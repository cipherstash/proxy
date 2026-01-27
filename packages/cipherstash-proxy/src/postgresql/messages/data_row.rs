use super::{BackendCode, NULL};
use crate::{
    error::{EncryptError, Error, ProtocolError},
    log::DECRYPT,
    postgresql::Column,
};
use bytes::{Buf, BufMut, BytesMut};
use cipherstash_client::eql::EqlCiphertext;
use std::io::Cursor;
use tracing::{debug, error};

#[derive(Debug, Clone)]
pub struct DataRow {
    pub columns: Vec<DataColumn>,
}

#[derive(Debug, Clone)]
pub struct DataColumn {
    bytes: Option<BytesMut>,
}

impl DataRow {
    pub fn as_ciphertext(
        &mut self,
        column_configuration: &Vec<Option<Column>>,
    ) -> Vec<Option<EqlCiphertext>> {
        let mut result = vec![];
        for (data_column, column_config) in self.columns.iter_mut().zip(column_configuration) {
            let encrypted = column_config
                .as_ref()
                .filter(|_| data_column.is_not_null())
                .and_then(|config| {
                    data_column
                        .try_into()
                        .inspect_err(|err| match err {
                            Error::Encrypt(EncryptError::ColumnIsNull) => {
                                debug!(target: DECRYPT, msg ="ColumnIsNull", ?config);
                                // Not an error, as you were
                                data_column.set_null();
                            }
                            _ => {
                                let err = EncryptError::ColumnCouldNotBeDeserialised {
                                    table: config.identifier.table.to_owned(),
                                    column: config.identifier.column.to_owned(),
                                };
                                error!(target: DECRYPT, msg = err.to_string());
                            }
                        })
                        .ok()
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
    pub fn is_not_null(&self) -> bool {
        self.bytes.is_some()
    }

    pub fn set_null(&mut self) {
        self.bytes = None;
    }

    pub fn rewrite(&mut self, b: &[u8]) {
        if let Some(ref mut bytes) = self.bytes {
            bytes.clear();
            bytes.extend_from_slice(b);
        }
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

impl TryFrom<&mut DataColumn> for EqlCiphertext {
    type Error = Error;

    fn try_from(col: &mut DataColumn) -> Result<Self, Error> {
        if let Some(bytes) = &col.bytes {
            if &bytes[0..=1] == b"(\"" {
                // Text encoding
                // Encrypted record is in the form ("{}")
                // json data can be extracted by dropping the first and last two bytes to remove (" and ")
                let start = 2;
                let end = bytes.len() - 2;
                let sliced = &bytes[start..end];

                let input = String::from_utf8_lossy(sliced).to_string();
                let input = input.replace("\"\"", "\"");

                match serde_json::from_str(&input) {
                    Ok(e) => return Ok(e),
                    Err(err) => {
                        debug!(target: DECRYPT, error = err.to_string());
                        return Err(err.into());
                    }
                }
            } else {
                // BINARY ENCODING
                // 12 bytes for the binary rowtype header
                // plus 1 byte for the jsonb header (value of 1)
                // [Int32] Number of fields (N)
                // [Int32] OID of the field’s type
                // [Int32] Length of the field (in bytes), or -1 for NULL

                let start = 4 + 4;
                let end = 4 + 4 + 4;

                let mut len_bytes = [0u8; 4]; // Create a fixed-size array
                len_bytes.copy_from_slice(&bytes[start..end]);

                let len = i32::from_be_bytes(len_bytes);

                if len == NULL {
                    return Err(EncryptError::ColumnIsNull.into());
                }

                let start = 12 + 1;
                let sliced = &bytes[start..];

                match serde_json::from_slice(sliced) {
                    Ok(e) => {
                        return Ok(e);
                    }
                    Err(err) => {
                        debug!(target: DECRYPT, error = err.to_string());
                        return Err(err.into());
                    }
                }
            }
        }

        Err(EncryptError::ColumnCouldNotBeParsed.into())
    }
}

#[cfg(test)]
mod tests {
    use super::DataRow;
    use crate::Identifier;
    use crate::{
        config::{LogConfig, LogLevel},
        log,
        postgresql::{messages::data_row::DataColumn, Column},
    };
    use bytes::BytesMut;
    use cipherstash_client::schema::{ColumnConfig, ColumnType};

    fn to_message(s: &[u8]) -> BytesMut {
        BytesMut::from(s)
    }

    fn column_config(column: &str) -> Option<Column> {
        let identifier = Identifier::new("encrypted", column);
        let config = ColumnConfig::build("column".to_string()).casts_as(ColumnType::SmallInt);
        let column = Column::new(identifier, config, None, eql_mapper::EqlTermVariant::Full);
        Some(column)
    }

    fn column_config_with_id(column: &str) -> Vec<Option<Column>> {
        vec![None, column_config(column)]
    }

    #[test]
    pub fn to_ciphertext_with_binary_encoding() {
        log::init(LogConfig::with_level(LogLevel::Debug));

        //  Binary
        // SELECT id, encrypted_text FROM encrypted WHERE id = $1
        let bytes = to_message(b"D\0\0\nR\0\x02\0\0\0\x08w\xaam\xf8Y$\x9dI\0\0\n<\0\0\0\x01\0\0\x0e\xda\0\0\n0\x01{\"b\": null, \"c\": \"mBbLbP2ww9ymEpm_yfj>@=^)JCqtLxcewai)Ilzx#HbC2p3F;dB`XP9af|s-igMjdMWLYPqYWAB#2|%<Q?A|Izg<&Cs4$4MtatzDN{_NWZFbsdA0=?)lF+VYZewzJaCBv4Uvy=7bie\", \"i\": {\"c\": \"encrypted_text\", \"t\": \"encrypted\"}, \"m\": [71, 1624, 929, 1339, 1764, 1380, 1256, 2018, 575, 470, 1792, 1684, 205, 894, 1365, 272, 814, 1333, 1971, 1942, 1335, 404, 1204, 638, 18, 1147, 1098, 1448, 403, 234, 647, 1982, 279, 1606, 826, 113, 652, 1287, 986, 1239, 1988, 358, 1589, 1775, 1997, 633, 369, 1744, 1700, 1149, 1641, 609, 1506, 1915, 630, 1045, 141, 815, 445, 145, 1758, 1772, 1162, 1761, 1619, 1328, 901, 1090, 637, 1529, 1181, 527, 388, 2015, 1317, 1019, 1369, 1340, 176, 936, 1716, 515, 1101, 656, 1737, 1858, 1633, 1849, 512, 1347, 1389, 700, 1336, 1253, 1396, 381, 35, 586, 581, 1877, 1226, 1273, 80, 1499, 433, 1649, 573, 1150, 1572, 1533, 1077], \"o\": [\"faa1f63cb6d36094d1aa50db6c0217eb447a987071119bb127f677b6a7ee0b4fe40eed7cd84e96e8a11bbe3ea14331f3ec4c8f149ce9d2b0253b4676c86557fcec4a5f8ca4e1ee081c66bf0a3cb594c6b5739f77f62fc5e76991869c23a97f01816cde3dfc24b2ca2fbb12b50fde324f18aa51718d681772bf9caf3c059a6748cbcaf4dd1c4fa026e47f4be75ce9046de508041645c0d48cddef735db92b4495a783b2a0f54d6723c959f74aae6fab62202f0d1f2cc2336ced18df80d12b4c6b5e504d2f6e21e12e2cc8ae620b9c714b8becbed3ed8d2b4ece3f0c911eeee4cba805098becdb041966faf06546cb48037153c3285f3d53f750c7cb7b1b6a985de296d0592b0bcb71687a09cd38d53979c5399245a85f9c8c5db68d14a2c2521795b8d670700e1ff324e0f46fe5338f63074adeba5a8a7d81b2693413ed97aa827b5a16ce9fa33ff9c2870465d992e367dfca76d957cbfb1062433825b83941f40b4d47cd65522c8634f4440b058dae20ec940eecaad70f46a81599ebec6c90735a51170f5456685307e9bdb5d9b94665c6b86985dcd95125\", \"b41d89a196a35252a965ce3c330eac369ead56e9f06e2016da4d6971fe0b8d6e677e1018e7a1bd2fa0b2c1faaa12650d678352ecc81f6be879213fe78b8004b87dd7dcadec59df4dcafdb3c9aa55dcb2cc2bcf2193574b201c9a1c14764d69716f63b0c1aa30a2846696f2a1c790ca2cb26370d7e20904a8748ea98a95ee3cbb95c5f342de4e71bb080b6e5cfcb730ba4c094304c759fe520fd59fa20eb1381bf9cb07b3a952a9cd0994ba49085475b605c67df0f5fb970fe20c343bdb6e4e90037656787cfd58622aa6f77026c57b66b95390f7560ed7dc640553e6219dea4015b3484e835241090d1bae888cbf946dc114680e36119a3aa95f64fb13c88dd6e9636898d423d82964dddec5311ec94a30b71eab561988be6cd07e6b7c18df9b4f0fffcc53cec5e883830ab868ee626d7c9bf6bbf8612eb2e0b472e223a06ddf920988630809e02cacf525e7533406f08ec7b192f2d78cb9d4acda1cd8e0d35898ff15b0b6c00d5dc4fa9e45f0666319e32a54d41da63b34cd83b2ccbb70db48ab3d3a959f2e24e40b899e334b8458430376cb7e28c8e673\"], \"s\": null, \"u\": \"962d77dfaf892b596b3255c022359e54f3e8dc8b21c3d1b32ebd05555f433192\", \"v\": 1, \"sv\": null, \"ocf\": null, \"ocv\": null}");
        let mut data_row = DataRow::try_from(&bytes).unwrap();

        let column_config = column_config_with_id("encrypted_text");
        let encrypted = data_row.as_ciphertext(&column_config);

        assert_eq!(encrypted.len(), 2);

        // Two rows
        assert!(encrypted[0].is_none());
        assert!(encrypted[1].is_some());

        assert_eq!(
            column_config[1].as_ref().unwrap().identifier,
            encrypted[1].as_ref().unwrap().identifier
        );
    }

    #[test]
    pub fn to_ciphertext_with_binary_encoding_and_null() {
        log::init(LogConfig::with_level(LogLevel::Debug));

        // Binary
        // encrypted_text IS NULL
        // SELECT id, encrypted_text FROM encrypted WHERE id = $1

        // let bytes = to_message(b"D\0\0\0\"\0\x02\0\0\0\x089\"\x88A\xe59\xb0\x13\0\0\0\x0c\0\0\0\x01\0\0\x0e\xda\xff\xff\xff\xff");
        let bytes = to_message(b"D\0\0\0\"\0\x02\0\0\0\x08>\xe6=<Yk\0\r\0\0\0\x0c\0\0\0\x01\0\0\x0e\xda\xff\xff\xff\xff");
        let mut data_row = DataRow::try_from(&bytes).unwrap();

        assert!(data_row.columns[1].bytes.is_some());

        let column_config = column_config_with_id("encrypted_text");
        let encrypted = data_row.as_ciphertext(&column_config);

        assert_eq!(encrypted.len(), 2);

        // Two rows
        assert!(encrypted[0].is_none());
        assert!(encrypted[1].is_none());

        // DataColumn has been NULLIFIED
        assert!(data_row.columns[1].bytes.is_none());
    }

    #[test]
    pub fn to_ciphertext_with_text_encoding() {
        log::init(LogConfig::with_level(LogLevel::Debug));

        // SELECT encrypted_jsonb FROM encrypted LIMIT 1
        let bytes = to_message(b"D\0\0\x03\xba\0\x01\0\0\x03\xb0(\"{\"\"b\"\": null, \"\"c\"\": \"\"mBbLR(BvRN1BF^PAFs!B^`U;mA>uOUiFLgDpZXhU#s#%c4wyi&Z7`(d0IxUty-cI#Yp%o~QFF39^sRf>4*EG{zlk;}ArEQ}NQHa9@;T73aPOSTpuh\"\", \"\"i\"\": {\"\"c\"\": \"\"encrypted_jsonb\"\", \"\"t\"\": \"\"encrypted\"\"}, \"\"m\"\": null, \"\"o\"\": null, \"\"s\"\": null, \"\"u\"\": null, \"\"v\"\": 1, \"\"sv\"\": [{\"\"b\"\": \"\"8067db44a848ab32c3056a3dbe4edf16\"\", \"\"c\"\": \"\"mBbLR(BvRN1BF^PAFs!B^`U;mA>uOUiFLgDpZXhU#s#%c4wyi&Z7`(d0IxUty-cI#Yp%o~QFF39^sRf>4*EG{zlk;}ArEQ}NQHa9@;T73aPOSTpuh\"\", \"\"m\"\": null, \"\"o\"\": null, \"\"s\"\": \"\"9493d6010fe7845d52149b697729c745\"\", \"\"u\"\": null, \"\"sv\"\": null, \"\"ocf\"\": null, \"\"ocv\"\": null}, {\"\"b\"\": null, \"\"c\"\": \"\"mBbLR(BvRN1BF^PAFs!B^`U;m8QkTKr|h>Q`^NbW(CC|>SD}UM=o%mz(Fw#LQFF39^sRf>4*EG{zlk;}ArEQ}NQHa9@;T73aPOSTpuh\"\", \"\"m\"\": null, \"\"o\"\": null, \"\"s\"\": \"\"b1f0e4bb3855bc33936ef1fddf532765\"\", \"\"u\"\": null, \"\"sv\"\": null, \"\"ocf\"\": null, \"\"ocv\"\": \"\"fbc7a11fc81f2a31c904c5b05572b054824e3b5f5ece78f1b711f93175f0a4a9726157cea247e107\"\"}], \"\"ocf\"\": null, \"\"ocv\"\": null}\")");
        let mut data_row = DataRow::try_from(&bytes).unwrap();

        assert!(data_row.columns[0].bytes.is_some());

        let column_config = vec![column_config("encrypted_jsonb")];
        let encrypted = data_row.as_ciphertext(&column_config);

        assert_eq!(encrypted.len(), 1);
        assert!(encrypted[0].is_some());

        assert_eq!(
            column_config[0].as_ref().unwrap().identifier,
            encrypted[0].as_ref().unwrap().identifier
        );
    }

    #[test]
    pub fn to_ciphertext_with_text_encoding_and_null() {
        log::init(LogConfig::with_level(LogLevel::Debug));

        // SELECT * FROM encrypted WHERE id = $1;
        // Only encrypted_text is NOT NULL
        let bytes = to_message(b"D\0\0\n\x91\0\n\0\0\0\n1297231342\xff\xff\xff\xff\0\0\nY(\"{\"\"b\"\": null, \"\"c\"\": \"\"mBbJ;S^xMu<v?;UyTSS~VfK;4C(U~uOiKbWSK*!hB3vi!C$luW$k`K6>@++(U20{lxK;qYYaDYF#30N~x;wyOUMoFOB9K!>A_9g9j@+M6V3wENqu#H8gDb9OZewzJaCBv4Uvy=7bie\"\", \"\"i\"\": {\"\"c\"\": \"\"encrypted_text\"\", \"\"t\"\": \"\"encrypted\"\"}, \"\"m\"\": [369, 381, 1758, 403, 35, 609, 1181, 1098, 1347, 1633, 1150, 815, 1997, 234, 1858, 656, 1335, 936, 1204, 630, 1764, 1328, 1649, 1396, 113, 1149, 1499, 1147, 586, 1942, 901, 1256, 1226, 1045, 637, 279, 1162, 1077, 1340, 1336, 1448, 700, 176, 1849, 1915, 1389, 71, 515, 633, 388, 1877, 1339, 1239, 638, 1365, 1380, 1273, 581, 1792, 1716, 145, 512, 814, 272, 1333, 1775, 1572, 1744, 2018, 433, 1641, 1529, 647, 1317, 652, 1606, 1737, 470, 826, 80, 929, 1700, 1619, 1253, 358, 1589, 1971, 1019, 1533, 1624, 573, 1684, 1287, 575, 1761, 527, 404, 1369, 894, 18, 1101, 986, 1772, 1090, 1506, 2015, 1988, 205, 141, 445, 1982], \"\"o\"\": [\"\"faa1f63cb6d36094d1aa50db6c0217eb447a987071119bb127f677b6a7ee0b4fe40eed7cd84e96e8a11bbe3ea14331f3ec4c8f149ce9d2b0253b4676c86557fcec4a5f8ca4e1ee081c66bf0a3cb594c6b5739f77f62fc5e76991869c23a97f01816cde3dfc24b2ca2fbb12b50fde324f18aa51718d681772bf9caf3c059a6748cbcaf4dd1c4fa02645d74699d7d265faf938c339f6cc8f57db9bd4cff8e03cae9e5d21a651b33525e86e335dff61520e8f23d7002f05fa186075a335fb7b2c740133b5a72760ccd216127d69983aa31a090a3b6ca56a48b6372cab60c979465d84dc94e5452c92517b643882fa82c22a26b4feaaa1b0ae8fcb989b10d0351fb3c9c5e56e719f820442612a67fff334438f3f5d35ff6db1b5f7a50670c7fec014f6fc19c352eb011911faf62a230e10c2d16f6c84b46cf9ee7eb1afb9c61a523891e31da2a18b445769d75c11873566dc8196d77e985423226bd1db10e4ce9eb10c2f69db7ce57d47281401617978d2bcfca23b9015b9e705615b8bf773daa87a18417f86e5338a7929fa4f10c6864af09870bfd9ddfb7848\"\", \"\"b41d89a196a35252a965ce3c330eac369ead56e9f06e2016da4d6971fe0b8d6e677e1018e7a1bd2fa0b2c1faaa12650d678352ecc81f6be879213fe78b8004b87dd7dcadec59df4dcafdb3c9aa55dcb2cc2bcf2193574b201c9a1c14764d69716f63b0c1aa30a2846696f2a1c790ca2cb26370d7e20904a8748ea98a95ee3cbb95c5f342de4e71bbf0262e84d59188ea72fe4449a16e7c73f88ed06b9cb724902a85d063c03e9b1a63dd18b9604625ca3cb8110d9c8f93e1771525c51b6ee092d554e84d61df5b557994f32191bb2b6801d9727fb707d5287e6c83d6b16763a6e66526baf80765a58d36df744be7872d2750eb28a86a519a21ee710f618c09cb2bd45f21e805ae4e11eb2987d7be31c32164d4f828fc35c389d516d0d6a54e25041985cffcb6124b4d3fa5b0ba91e19d60e3102370e9c1c768df1b427c682304a1dfdea2d3e514db22057f43d8121b8daf7c434831e5b618bbca9f4e198741927bdc168e4703fb1f703957f7b70491e06bec4adee19d29ef5e938695e1d49ef50ceef0a9c3e46bd8fe309e013e5ea0d35c5ebf3dddd97573\"\"], \"\"s\"\": null, \"\"u\"\": \"\"962d77dfaf892b596b3255c022359e54f3e8dc8b21c3d1b32ebd05555f433192\"\", \"\"v\"\": 1, \"\"sv\"\": null, \"\"ocf\"\": null, \"\"ocv\"\": null}\")\xff\xff\xff\xff\xff\xff\xff\xff\xff\xff\xff\xff\xff\xff\xff\xff\xff\xff\xff\xff\xff\xff\xff\xff\xff\xff\xff\xff");

        let mut data_row = DataRow::try_from(&bytes).unwrap();

        assert!(data_row.columns[0].bytes.is_some());

        let column_config = vec![
            None,
            None,
            column_config("encrypted_text"),
            column_config("encrypted_bool"),
            column_config("encrypted_int2"),
            column_config("encrypted_int4"),
            column_config("encrypted_int8"),
            column_config("encrypted_float8"),
            column_config("encrypted_date"),
            column_config("encrypted_jsonb"),
        ];

        let encrypted = data_row.as_ciphertext(&column_config);

        assert_eq!(encrypted.len(), 10);

        assert!(encrypted[0].is_none());
        assert!(encrypted[1].is_none());
        assert!(encrypted[2].is_some()); // <-- Some
        assert!(encrypted[3].is_none());
        // etc

        assert_eq!(
            column_config[2].as_ref().unwrap().identifier,
            encrypted[2].as_ref().unwrap().identifier
        );
    }

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
