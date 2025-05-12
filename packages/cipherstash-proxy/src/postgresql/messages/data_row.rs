use super::{maybe_json, maybe_jsonb, BackendCode, NULL};
use crate::{
    eql,
    error::{EncryptError, Error, ProtocolError},
    log::DECRYPT,
    postgresql::{data, Column},
};
use bytes::{Buf, BufMut, BytesMut};
use std::io::Cursor;
use tracing::{debug, error, info, warn};
use winnow::{
    ascii::{alpha1, escaped, escaped_transform},
    combinator::{alt, delimited, eof, not, opt, repeat_till, terminated},
    token::{any, literal, rest, take_until, take_while},
    ModalResult, Parser,
};

#[derive(Debug, Clone)]
pub struct DataRow {
    pub columns: Vec<DataColumn>,
}

#[derive(Debug, Clone)]
pub struct DataColumn {
    bytes: Option<BytesMut>,
}

impl DataRow {
    pub fn to_ciphertext(
        &self,
        column_configuration: &Vec<Option<Column>>,
    ) -> Vec<Option<eql::EqlEncrypted>> {
        let mut result = vec![];
        for (data_column, column_config) in self.columns.iter().zip(column_configuration) {
            let encrypted = column_config.as_ref().and_then(|config| {
                warn!(target: DECRYPT, msg = "to_ciphertext");
                if data_column.is_not_null() {
                    data_column
                        .try_into()
                        .inspect_err(|err: &Error| {
                            debug!(target: DECRYPT, msg = err.to_string(), error = ?err);

                            let err = EncryptError::ColumnCouldNotBeDeserialised {
                                table: config.identifier.table.to_owned(),
                                column: config.identifier.column.to_owned(),
                            };

                            error!(target: DECRYPT, msg = err.to_string());
                        })
                        .ok()
                } else {
                    None
                }
            });
            result.push(encrypted);
        }

        result
    }

    pub fn column_count(&self) -> usize {
        self.columns.len()
    }

    fn len_of_columns(&self) -> usize {
        let column_len_size = size_of::<i32>(); // len of column len

        self.columns
            .iter()
            .map(|col| column_len_size + col.bytes.as_ref().map(|b| b.len()).unwrap_or(0))
            .sum()
    }

    pub fn rewrite(&mut self, plaintexts: &[Option<BytesMut>]) -> Result<(), Error> {
        for (idx, pt) in plaintexts.iter().enumerate() {
            if let Some(bytes) = pt {
                self.columns[idx].rewrite(bytes);
            }
        }
        Ok(())
    }
}

impl DataColumn {
    #[deprecated(note = "unused?")]
    pub fn get_data(&self) -> Option<Vec<u8>> {
        self.bytes.as_ref().map(|b| b.to_vec())
    }

    pub fn is_not_null(&self) -> bool {
        self.bytes.is_some()
    }

    ///
    ///
    /// Note on quotes in composite types: https://www.postgresql.org/docs/current/rowtypes.html
    /// -> "Double quotes and backslashes embedded in field values will be doubled."
    ///
    pub fn parse(bytes: &BytesMut) -> Result<eql::EqlEncrypted, Error> {
        let input = String::from_utf8_lossy(bytes.as_ref()).to_string();

        warn!(target: DECRYPT, msg = "parse_column", input = ?input);

        let input = input.replace("\"\"", "\"");

        warn!(target: DECRYPT, msg = "parse_column", input_escaped = ?input);

        match parse_column.parse(&input) {
            Ok(s) => {
                warn!(target: DECRYPT, parsed = s);
                let e = serde_json::from_str(&s)?;
                Ok(e)
            }
            Err(err) => {
                error!(target: DECRYPT, ?err);
                Err(EncryptError::ColumnCouldNotBeParsed.into())
            }
        }
    }

    pub fn rewrite(&mut self, b: &[u8]) {
        if let Some(ref mut bytes) = self.bytes {
            bytes.clear();
            bytes.extend_from_slice(b);
        }
    }

    #[deprecated(note = "unused?")]
    pub fn maybe_ciphertext(&self) -> bool {
        self.bytes
            .as_ref()
            .is_some_and(|b| maybe_jsonb(b) || maybe_json(b))
    }

    ///
    /// If the json format looks binary, returns a reference to the bytes without the jsonb header byte
    ///
    #[deprecated(note = "unused?")]
    pub fn json_bytes(&self) -> Option<&[u8]> {
        self.bytes.as_ref().and_then(|b| {
            if maybe_jsonb(b) {
                Some(&b[1..])
            } else if maybe_json(b) {
                Some(&b[0..])
            } else {
                None
            }
        })
    }
}

impl TryFrom<&BytesMut> for DataRow {
    type Error = Error;

    fn try_from(buf: &BytesMut) -> Result<DataRow, Error> {
        let mut cursor = Cursor::new(buf);

        let code = cursor.get_u8();

        if BackendCode::from(code) != BackendCode::DataRow {
            return Err(ProtocolError::UnexpectedMessageCode {
                expected: BackendCode::DataRow.into(),
                received: code as char,
            }
            .into());
        }

        let _len = cursor.get_i32();

        let num_columns = cursor.get_i16();

        let mut columns = Vec::new();
        for _ in 0..num_columns {
            let len = cursor.get_i32();

            if len == NULL {
                columns.push(DataColumn { bytes: None });
            } else {
                // Len of json is len of column minus 4 bytes
                let len = len as usize;

                let mut bytes = BytesMut::with_capacity(len);
                bytes.resize(len, 0);
                cursor.copy_to_slice(&mut bytes);

                columns.push(DataColumn { bytes: Some(bytes) });
            }
        }

        Ok(DataRow { columns })
    }
}

impl TryFrom<DataRow> for BytesMut {
    type Error = Error;

    fn try_from(data_row: DataRow) -> Result<BytesMut, Error> {
        let mut bytes = BytesMut::new();

        let len = size_of::<i32>() // len of len
                        + size_of::<i16>() // num columns
                        + data_row.len_of_columns(); // len data columns

        bytes.put_u8(BackendCode::DataRow.into());
        bytes.put_i32(len as i32);
        bytes.put_i16(data_row.columns.len() as i16);

        for col in data_row.columns.into_iter() {
            let b = BytesMut::try_from(col)?;
            bytes.put_slice(&b);
        }

        Ok(bytes)
    }
}

impl TryFrom<DataColumn> for BytesMut {
    type Error = Error;

    fn try_from(data_column: DataColumn) -> Result<BytesMut, Error> {
        let mut bytes = BytesMut::new();

        if let Some(data) = data_column.bytes {
            bytes.put_i32(data.len() as i32);
            bytes.put_slice(&data);
        } else {
            bytes.put_i32(NULL);
        }

        Ok(bytes)
    }
}

impl TryFrom<&DataColumn> for eql::EqlEncrypted {
    type Error = Error;

    fn try_from(col: &DataColumn) -> Result<Self, Error> {
        debug!(target: DECRYPT, data_column = ?col);

        if let Some(bytes) = &col.bytes {
            // debug!(target: DECRYPT, msg = "DataColumn Bytes", ?bytes);
            // debug!(target: DECRYPT, msg = "DataColumn MaybeJsonB Bytes", maybe_jsonb = ?maybe_jsonb(bytes));

            // Encrypted record is in the form ("{}")
            // json data can be extracted by dropping the first and last two bytes to remove (" and ")
            let start = 2;
            let end = bytes.len() - 2;
            let sliced = &bytes[start..end];

            let input = String::from_utf8_lossy(sliced.as_ref()).to_string();
            let input = input.replace("\"\"", "\"");

            debug!(target: DECRYPT, ?input);

            // match serde_json::from_slice(sliced) {
            match serde_json::from_str(&input) {
                Ok(e) => return Ok(e),
                Err(err) => {
                    debug!(target: DECRYPT, msg = "Could not convert DataColumn to Ciphertext", error = err.to_string());
                    return Err(err.into());
                }
            }
        }

        return Err(EncryptError::ColumnCouldNotBeParsed.into());
    }
}

fn remove_quotes<'s>(input: &mut &'s str) -> ModalResult<&'s str> {
    literal("\"\"").value("\"").parse_next(input)
}

///
/// Encrypted value is a PostgreSQL tuple in form "(\"{ ... }\")"
///
///
///
// fn end_of_encrypted<'s>(input: &mut &'s str) -> ModalResult<&'s str> {
//     (("\")", eof)).parse_next(input)
// }

fn take_json<'s>(input: &mut &'s str) -> ModalResult<&'s str> {
    info!("{:?}", input);

    take_until(1.., "\")").parse_next(input)
}

// fn until_eof<'i>(s: &mut &'i str) -> ModalResult<&'i str> {
//     terminated(take_until(1.., ")"), ")").parse_next(s)
// }

///
/// eql_v1_encrypted bytes
///   (\"{\"  ... }\")
///
/// Characters `"`, and "(" are valid chars in Ascii85 encoding, so need to disambiguate
///
fn parse_column<'s>(input: &mut &'s str) -> ModalResult<&'s str> {
    // terminated(delimited("(\"", take_json, "\")"), eof).parse_next(input)
    delimited("(\"", take_json, "\")").parse_next(input)
}

pub fn maybe_encrypted(bytes: &BytesMut) -> bool {
    let b = bytes.as_ref();

    debug!(target: DECRYPT, msg = "maybe_encrypted", bytes = ?b);

    let first = b[0];

    debug!(target: DECRYPT, msg = "maybe_encrypted", first, maybe = (first == b'('));

    first == b'('
}

#[cfg(test)]
mod tests {
    use super::DataRow;
    use crate::config::LogLevel;
    use crate::postgresql::messages::data_row::{parse_column, take_json};
    use crate::{config::LogConfig, log, postgresql::messages::data_row::DataColumn};
    use crate::{eql, EqlEncrypted, Identifier};
    use bytes::{Buf, BytesMut};
    use cipherstash_client::zerokms::EncryptedRecord;
    use recipher::key::Iv;
    use serde_json::Value;
    use tracing::{error, info};
    use uuid::Uuid;
    use winnow::token::take_until;
    use winnow::{combinator::delimited, ModalResult, Parser};

    fn to_message(s: &[u8]) -> BytesMut {
        BytesMut::from(s)
    }

    fn record() -> EncryptedRecord {
        EncryptedRecord {
            iv: Iv::default(),
            ciphertext: vec![1; 32],
            tag: vec![1; 16],
            descriptor: "users/name".to_string(),
            dataset_id: Some(Uuid::new_v4()),
        }
    }

    #[test]
    pub fn parse_column_string() {
        log::init(LogConfig::default());

        let input = "(\"{}\")";
        let result = parse_column.parse(input).unwrap();
        assert_eq!(result, "{}");

        let input = "(\"{\"c\": \"ciphertext\"}\")";
        let result = parse_column.parse(input).unwrap();
        assert_eq!(result, "{\"c\": \"ciphertext\"}");

        // JSON data contains `")`
        let input = "(\"{\"c\": \"cipher\")text\"}\")";
        let result = parse_column.parse(input).unwrap();
        assert_eq!(result, "{\"c\": \"cipher\")text\"}");

        info!("{:?}", result);
    }

    #[test]
    pub fn parse_encrypted_column() {
        log::init(LogConfig::with_level(LogLevel::Debug));

        // SELECT encrypted_jsonb FROM encrypted LIMIT 1
        let bytes = to_message(b"D\0\0\x03\xba\0\x01\0\0\x03\xb0(\"{\"\"b\"\": null, \"\"c\"\": \"\"mBbLR(BvRN1BF^PAFs!B^`U;mA>uOUiFLgDpZXhU#s#%c4wyi&Z7`(d0IxUty-cI#Yp%o~QFF39^sRf>4*EG{zlk;}ArEQ}NQHa9@;T73aPOSTpuh\"\", \"\"i\"\": {\"\"c\"\": \"\"encrypted_jsonb\"\", \"\"t\"\": \"\"encrypted\"\"}, \"\"m\"\": null, \"\"o\"\": null, \"\"s\"\": null, \"\"u\"\": null, \"\"v\"\": 1, \"\"sv\"\": [{\"\"b\"\": \"\"8067db44a848ab32c3056a3dbe4edf16\"\", \"\"c\"\": \"\"mBbLR(BvRN1BF^PAFs!B^`U;mA>uOUiFLgDpZXhU#s#%c4wyi&Z7`(d0IxUty-cI#Yp%o~QFF39^sRf>4*EG{zlk;}ArEQ}NQHa9@;T73aPOSTpuh\"\", \"\"m\"\": null, \"\"o\"\": null, \"\"s\"\": \"\"9493d6010fe7845d52149b697729c745\"\", \"\"u\"\": null, \"\"sv\"\": null, \"\"ocf\"\": null, \"\"ocv\"\": null}, {\"\"b\"\": null, \"\"c\"\": \"\"mBbLR(BvRN1BF^PAFs!B^`U;m8QkTKr|h>Q`^NbW(CC|>SD}UM=o%mz(Fw#LQFF39^sRf>4*EG{zlk;}ArEQ}NQHa9@;T73aPOSTpuh\"\", \"\"m\"\": null, \"\"o\"\": null, \"\"s\"\": \"\"b1f0e4bb3855bc33936ef1fddf532765\"\", \"\"u\"\": null, \"\"sv\"\": null, \"\"ocf\"\": null, \"\"ocv\"\": \"\"fbc7a11fc81f2a31c904c5b05572b054824e3b5f5ece78f1b711f93175f0a4a9726157cea247e107\"\"}], \"\"ocf\"\": null, \"\"ocv\"\": null}\")");
        let data_row = DataRow::try_from(&bytes).unwrap();

        // let bytes = data_row.columns.first().unwrap().bytes.as_ref().unwrap();
        // let e = DataColumn::parse(bytes).unwrap();

        let col = data_row.columns.first().unwrap();
        let e: EqlEncrypted = col.try_into().unwrap();

        let expected = Identifier::new("encrypted", "encrypted_jsonb");

        assert_eq!(e.identifier, expected);

        // SELECT * FROM encrypted WHERE id = $1;
        // Only encrypted_text is NOT NULL
        let bytes = to_message(b"D\0\0\n\x91\0\n\0\0\0\n1297231342\xff\xff\xff\xff\0\0\nY(\"{\"\"b\"\": null, \"\"c\"\": \"\"mBbJ;S^xMu<v?;UyTSS~VfK;4C(U~uOiKbWSK*!hB3vi!C$luW$k`K6>@++(U20{lxK;qYYaDYF#30N~x;wyOUMoFOB9K!>A_9g9j@+M6V3wENqu#H8gDb9OZewzJaCBv4Uvy=7bie\"\", \"\"i\"\": {\"\"c\"\": \"\"encrypted_text\"\", \"\"t\"\": \"\"encrypted\"\"}, \"\"m\"\": [369, 381, 1758, 403, 35, 609, 1181, 1098, 1347, 1633, 1150, 815, 1997, 234, 1858, 656, 1335, 936, 1204, 630, 1764, 1328, 1649, 1396, 113, 1149, 1499, 1147, 586, 1942, 901, 1256, 1226, 1045, 637, 279, 1162, 1077, 1340, 1336, 1448, 700, 176, 1849, 1915, 1389, 71, 515, 633, 388, 1877, 1339, 1239, 638, 1365, 1380, 1273, 581, 1792, 1716, 145, 512, 814, 272, 1333, 1775, 1572, 1744, 2018, 433, 1641, 1529, 647, 1317, 652, 1606, 1737, 470, 826, 80, 929, 1700, 1619, 1253, 358, 1589, 1971, 1019, 1533, 1624, 573, 1684, 1287, 575, 1761, 527, 404, 1369, 894, 18, 1101, 986, 1772, 1090, 1506, 2015, 1988, 205, 141, 445, 1982], \"\"o\"\": [\"\"faa1f63cb6d36094d1aa50db6c0217eb447a987071119bb127f677b6a7ee0b4fe40eed7cd84e96e8a11bbe3ea14331f3ec4c8f149ce9d2b0253b4676c86557fcec4a5f8ca4e1ee081c66bf0a3cb594c6b5739f77f62fc5e76991869c23a97f01816cde3dfc24b2ca2fbb12b50fde324f18aa51718d681772bf9caf3c059a6748cbcaf4dd1c4fa02645d74699d7d265faf938c339f6cc8f57db9bd4cff8e03cae9e5d21a651b33525e86e335dff61520e8f23d7002f05fa186075a335fb7b2c740133b5a72760ccd216127d69983aa31a090a3b6ca56a48b6372cab60c979465d84dc94e5452c92517b643882fa82c22a26b4feaaa1b0ae8fcb989b10d0351fb3c9c5e56e719f820442612a67fff334438f3f5d35ff6db1b5f7a50670c7fec014f6fc19c352eb011911faf62a230e10c2d16f6c84b46cf9ee7eb1afb9c61a523891e31da2a18b445769d75c11873566dc8196d77e985423226bd1db10e4ce9eb10c2f69db7ce57d47281401617978d2bcfca23b9015b9e705615b8bf773daa87a18417f86e5338a7929fa4f10c6864af09870bfd9ddfb7848\"\", \"\"b41d89a196a35252a965ce3c330eac369ead56e9f06e2016da4d6971fe0b8d6e677e1018e7a1bd2fa0b2c1faaa12650d678352ecc81f6be879213fe78b8004b87dd7dcadec59df4dcafdb3c9aa55dcb2cc2bcf2193574b201c9a1c14764d69716f63b0c1aa30a2846696f2a1c790ca2cb26370d7e20904a8748ea98a95ee3cbb95c5f342de4e71bbf0262e84d59188ea72fe4449a16e7c73f88ed06b9cb724902a85d063c03e9b1a63dd18b9604625ca3cb8110d9c8f93e1771525c51b6ee092d554e84d61df5b557994f32191bb2b6801d9727fb707d5287e6c83d6b16763a6e66526baf80765a58d36df744be7872d2750eb28a86a519a21ee710f618c09cb2bd45f21e805ae4e11eb2987d7be31c32164d4f828fc35c389d516d0d6a54e25041985cffcb6124b4d3fa5b0ba91e19d60e3102370e9c1c768df1b427c682304a1dfdea2d3e514db22057f43d8121b8daf7c434831e5b618bbca9f4e198741927bdc168e4703fb1f703957f7b70491e06bec4adee19d29ef5e938695e1d49ef50ceef0a9c3e46bd8fe309e013e5ea0d35c5ebf3dddd97573\"\"], \"\"s\"\": null, \"\"u\"\": \"\"962d77dfaf892b596b3255c022359e54f3e8dc8b21c3d1b32ebd05555f433192\"\", \"\"v\"\": 1, \"\"sv\"\": null, \"\"ocf\"\": null, \"\"ocv\"\": null}\")\xff\xff\xff\xff\xff\xff\xff\xff\xff\xff\xff\xff\xff\xff\xff\xff\xff\xff\xff\xff\xff\xff\xff\xff\xff\xff\xff\xff");
        let data_row = DataRow::try_from(&bytes).unwrap();

        // Columns: id, plaintext, encrypted_text
        // let bytes = data_row.columns[2].bytes.as_ref().unwrap();

        // let e = DataColumn::parse(bytes).unwrap();
        let col = &data_row.columns[2];
        let e: EqlEncrypted = col.try_into().unwrap();
        let expected = Identifier::new("encrypted", "encrypted_text");

        assert_eq!(e.identifier, expected);
    }

    // #[test]
    // pub fn parse_encrypted_column_with_encoding_conflict() {
    //     log::init(LogConfig::default());

    //     // // SELECT * FROM encrypted WHERE id = $1;
    //     // // Only encrypted_text is NOT NULL
    //     // // Ciphertext has been edited to include `"")` which are valid Ascii85 chars
    //     // // The sequence `")` is also the end of the row encoding,
    //     // // The parser should ignore any occurences inside the JSONB body
    //     let bytes = to_message(b"D\0\0\n\x91\0\n\0\0\0\n1297231342\xff\xff\xff\xff\0\0\nY(\"{\"\"b\"\": null, \"\"c\"\": \"\"mBbJ;S^xMu<v?;UyTSS~VfK;4\"(U~uOiKbWSK*!hB3vi!C$luW$k`K6>@++(U20{lxK;qYYaDYF#30N~x;wyOUMoFOB9K!>A_9g9j@+M6V3wENqu#H8gDb9OZewzJaCBv4Uvy=7bie\"\", \"\"i\"\": {\"\"c\"\": \"\"encrypted_text\"\", \"\"t\"\": \"\"encrypted\"\"}, \"\"m\"\": [369, 381, 1758, 403, 35, 609, 1181, 1098, 1347, 1633, 1150, 815, 1997, 234, 1858, 656, 1335, 936, 1204, 630, 1764, 1328, 1649, 1396, 113, 1149, 1499, 1147, 586, 1942, 901, 1256, 1226, 1045, 637, 279, 1162, 1077, 1340, 1336, 1448, 700, 176, 1849, 1915, 1389, 71, 515, 633, 388, 1877, 1339, 1239, 638, 1365, 1380, 1273, 581, 1792, 1716, 145, 512, 814, 272, 1333, 1775, 1572, 1744, 2018, 433, 1641, 1529, 647, 1317, 652, 1606, 1737, 470, 826, 80, 929, 1700, 1619, 1253, 358, 1589, 1971, 1019, 1533, 1624, 573, 1684, 1287, 575, 1761, 527, 404, 1369, 894, 18, 1101, 986, 1772, 1090, 1506, 2015, 1988, 205, 141, 445, 1982], \"\"o\"\": [\"\"faa1f63cb6d36094d1aa50db6c0217eb447a987071119bb127f677b6a7ee0b4fe40eed7cd84e96e8a11bbe3ea14331f3ec4c8f149ce9d2b0253b4676c86557fcec4a5f8ca4e1ee081c66bf0a3cb594c6b5739f77f62fc5e76991869c23a97f01816cde3dfc24b2ca2fbb12b50fde324f18aa51718d681772bf9caf3c059a6748cbcaf4dd1c4fa02645d74699d7d265faf938c339f6cc8f57db9bd4cff8e03cae9e5d21a651b33525e86e335dff61520e8f23d7002f05fa186075a335fb7b2c740133b5a72760ccd216127d69983aa31a090a3b6ca56a48b6372cab60c979465d84dc94e5452c92517b643882fa82c22a26b4feaaa1b0ae8fcb989b10d0351fb3c9c5e56e719f820442612a67fff334438f3f5d35ff6db1b5f7a50670c7fec014f6fc19c352eb011911faf62a230e10c2d16f6c84b46cf9ee7eb1afb9c61a523891e31da2a18b445769d75c11873566dc8196d77e985423226bd1db10e4ce9eb10c2f69db7ce57d47281401617978d2bcfca23b9015b9e705615b8bf773daa87a18417f86e5338a7929fa4f10c6864af09870bfd9ddfb7848\"\", \"\"b41d89a196a35252a965ce3c330eac369ead56e9f06e2016da4d6971fe0b8d6e677e1018e7a1bd2fa0b2c1faaa12650d678352ecc81f6be879213fe78b8004b87dd7dcadec59df4dcafdb3c9aa55dcb2cc2bcf2193574b201c9a1c14764d69716f63b0c1aa30a2846696f2a1c790ca2cb26370d7e20904a8748ea98a95ee3cbb95c5f342de4e71bbf0262e84d59188ea72fe4449a16e7c73f88ed06b9cb724902a85d063c03e9b1a63dd18b9604625ca3cb8110d9c8f93e1771525c51b6ee092d554e84d61df5b557994f32191bb2b6801d9727fb707d5287e6c83d6b16763a6e66526baf80765a58d36df744be7872d2750eb28a86a519a21ee710f618c09cb2bd45f21e805ae4e11eb2987d7be31c32164d4f828fc35c389d516d0d6a54e25041985cffcb6124b4d3fa5b0ba91e19d60e3102370e9c1c768df1b427c682304a1dfdea2d3e514db22057f43d8121b8daf7c434831e5b618bbca9f4e198741927bdc168e4703fb1f703957f7b70491e06bec4adee19d29ef5e938695e1d49ef50ceef0a9c3e46bd8fe309e013e5ea0d35c5ebf3dddd97573\"\"], \"\"s\"\": null, \"\"u\"\": \"\"962d77dfaf892b596b3255c022359e54f3e8dc8b21c3d1b32ebd05555f433192\"\", \"\"v\"\": 1, \"\"sv\"\": null, \"\"ocf\"\": null, \"\"ocv\"\": null}\")\xff\xff\xff\xff\xff\xff\xff\xff\xff\xff\xff\xff\xff\xff\xff\xff\xff\xff\xff\xff\xff\xff\xff\xff\xff\xff\xff\xff");
    //     let data_row = DataRow::try_from(&bytes).unwrap();

    //     // Columns: id, plaintext, encrypted_text
    //     let col = &data_row.columns[2];
    //     let e: EqlEncrypted = col.try_into().unwrap();

    //     let expected = Identifier::new("encrypted", "encrypted_text");

    //     assert_eq!(e.identifier, expected);
    // }

    #[test]
    pub fn parse_data_row() {
        log::init(LogConfig::with_level(LogLevel::Debug));

        let messages = vec![
            to_message(b"D\0\0\0\x0e\0\x01\0\0\0\x04\0\0\x1e\xa2"),
            // SELECT encrypted_jsonb FROM encrypted LIMIT 1
            to_message(b"D\0\0\x03\xba\0\x01\0\0\x03\xb0(\"{\"\"b\"\": null, \"\"c\"\": \"\"mBbLR(BvRN1BF^PAFs!B^`U;mA>uOUiFLgDpZXhU#s#%c4wyi&Z7`(d0IxUty-cI#Yp%o~QFF39^sRf>4*EG{zlk;}ArEQ}NQHa9@;T73aPOSTpuh\"\", \"\"i\"\": {\"\"c\"\": \"\"encrypted_jsonb\"\", \"\"t\"\": \"\"encrypted\"\"}, \"\"m\"\": null, \"\"o\"\": null, \"\"s\"\": null, \"\"u\"\": null, \"\"v\"\": 1, \"\"sv\"\": [{\"\"b\"\": \"\"8067db44a848ab32c3056a3dbe4edf16\"\", \"\"c\"\": \"\"mBbLR(BvRN1BF^PAFs!B^`U;mA>uOUiFLgDpZXhU#s#%c4wyi&Z7`(d0IxUty-cI#Yp%o~QFF39^sRf>4*EG{zlk;}ArEQ}NQHa9@;T73aPOSTpuh\"\", \"\"m\"\": null, \"\"o\"\": null, \"\"s\"\": \"\"9493d6010fe7845d52149b697729c745\"\", \"\"u\"\": null, \"\"sv\"\": null, \"\"ocf\"\": null, \"\"ocv\"\": null}, {\"\"b\"\": null, \"\"c\"\": \"\"mBbLR(BvRN1BF^PAFs!B^`U;m8QkTKr|h>Q`^NbW(CC|>SD}UM=o%mz(Fw#LQFF39^sRf>4*EG{zlk;}ArEQ}NQHa9@;T73aPOSTpuh\"\", \"\"m\"\": null, \"\"o\"\": null, \"\"s\"\": \"\"b1f0e4bb3855bc33936ef1fddf532765\"\", \"\"u\"\": null, \"\"sv\"\": null, \"\"ocf\"\": null, \"\"ocv\"\": \"\"fbc7a11fc81f2a31c904c5b05572b054824e3b5f5ece78f1b711f93175f0a4a9726157cea247e107\"\"}], \"\"ocf\"\": null, \"\"ocv\"\": null}\")"),
        ];

        for bytes in messages {
            let expected = bytes.clone();

            let data_row = DataRow::try_from(&bytes).unwrap();

            let bytes = BytesMut::try_from(data_row).unwrap();
            assert_eq!(bytes, expected);
        }
    }

    #[test]
    pub fn parse_data_row_with_columns() {
        let bytes = to_message(
            b"D\0\0\09\0\x03\0\0\0\x08blahvtha\0\0\0\x0242\0\0\0\x1d2023-12-16 01:52:25.031985+00",
        );

        let data_row = DataRow::try_from(&bytes).unwrap();

        let data_col = data_row.columns.first().unwrap();

        let buf: &[u8] = data_col.bytes.as_ref().unwrap();
        let value = String::from_utf8_lossy(buf).to_string();
        assert_eq!(value, "blahvtha");
    }

    #[test]
    pub fn parse_data_row_with_null_column() {
        let bytes = to_message(b"D\0\0\0\n\0\x01\xff\xff\xff\xff");

        let data_row = DataRow::try_from(&bytes).unwrap();

        let data_col = data_row.columns.first().unwrap();

        assert_eq!(data_col.bytes, None);
    }

    #[test]
    pub fn data_row_column_len() {
        let column = DataColumn { bytes: None };
        let data_row = DataRow {
            columns: vec![column],
        };
        assert_eq!(data_row.len_of_columns(), 4);

        let data = BytesMut::from("");
        let column = DataColumn { bytes: Some(data) };
        let data_row = DataRow {
            columns: vec![column],
        };
        assert_eq!(data_row.len_of_columns(), 4);

        let data = BytesMut::from("blah");
        let column = DataColumn { bytes: Some(data) };
        let data_row = DataRow {
            columns: vec![column],
        };
        assert_eq!(data_row.len_of_columns(), 8);

        let mut columns = Vec::new();
        for _ in 1..5 {
            let data = BytesMut::from("blah");
            let column = DataColumn { bytes: Some(data) };
            columns.push(column);
        }
        let data_row = DataRow { columns };
        assert_eq!(data_row.len_of_columns(), 32);
    }
}
