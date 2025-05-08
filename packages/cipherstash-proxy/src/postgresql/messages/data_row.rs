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
    combinator::{alt, delimited, opt},
    token::{any, literal, take_until},
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
    pub fn get_data(&self) -> Option<Vec<u8>> {
        self.bytes.as_ref().map(|b| b.to_vec())
    }

    pub fn is_not_null(&self) -> bool {
        self.bytes.is_some()
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

            warn!(target: DECRYPT, msg = "DataColumn len", len);
            warn!(target: DECRYPT, msg = "DataColumn bytes", ?cursor);

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

impl TryFrom<&DataColumn> for eql::EqlEncrypted {
    type Error = Error;

    fn try_from(col: &DataColumn) -> Result<Self, Error> {
        debug!(target: DECRYPT, data_column = ?col);

        if let Some(bytes) = &col.bytes {
            debug!(target: DECRYPT, msg = "DataColumn Bytes", ?bytes);

            debug!(target: DECRYPT, msg = "DataColumn MaybeJsonB Bytes", maybe_jsonb = ?maybe_jsonb(bytes));

            if maybe_encrypted(bytes) {
                match parse_column(bytes) {
                    Ok(e) => return Ok(e),
                    Err(err) => {
                        debug!(target: DECRYPT, msg = "Could not convert DataColumn to Ciphertext", error = err.to_string());
                        return Err(err);
                    }
                }
            }
        }

        return Err(EncryptError::ColumnCouldNotBeParsed.into());
    }
}

fn remove_quotes<'s>(input: &mut &'s str) -> ModalResult<&'s str> {
    literal("\"\"").value("\"").parse_next(input)
}

fn take_json<'s>(input: &mut &'s str) -> ModalResult<&'s str> {
    take_until(1.., "\")").parse_next(input)
}

fn _parse_column<'s>(input: &mut &'s str) -> ModalResult<&'s str> {
    delimited("(\"", take_json, "\")").parse_next(input)
}

pub fn maybe_encrypted(bytes: &BytesMut) -> bool {
    let b = bytes.as_ref();

    debug!(target: DECRYPT, msg = "maybe_encrypted", bytes = ?b);

    let first = b[0];
    debug!(target: DECRYPT, msg = "maybe_encrypted", first, maybe = (first == b'('));

    first == b'('
}

fn parse_column(bytes: &BytesMut) -> Result<eql::EqlEncrypted, Error> {
    let input = String::from_utf8_lossy(bytes.as_ref()).to_string();

    debug!(target: DECRYPT, msg = "parse_column", input = ?input);

    let input = input.replace("\"\"", "\"");

    match _parse_column.parse(&input) {
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

#[cfg(test)]
mod tests {
    use super::DataRow;
    use crate::eql;
    use crate::postgresql::messages::data_row::parse_column;
    use crate::{config::LogConfig, log, postgresql::messages::data_row::DataColumn};
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
    pub fn data_column_parser() {
        log::init(LogConfig::default());

        // ARRAY of eql_v1_encrypted
        // SELECT ARRAY['{}'::jsonb::eql_v1_encrypted, '{}'::jsonb::eql_v1_encrypted];
        // b"D\0\0\0\x19\0\x01\0\0\0\x0f{\"({})\",\"({})\"}"

        // let r = parser().parse("\"hello\"");
        // let input = "(\"{}\")";
        // let s = parse_column(input).unwrap();
        // info!("{:?}", s);
        // assert_eq!(s, "{}");

        let input = "(\"{\"\"a\"\": 1}\")";
        // let s = parse_column(input).unwrap();
        // info!("{:?}", s);
        // assert_eq!(s, "{\"a\": 1}");

        // let input = "(\"{\"a\": 1}\")";
        // let s = parse_column(input).unwrap();
        // info!("{:?}", s);
        // assert_eq!(s, "{\"a\": 1}");

        let input = "(\"{\"\"b\"\": null, \"\"c\"\": \"\"mBbKJso20j8T)d)m<%^V#KrZ*CmRG{1Sa)(pJ-#ACnj8VuoPCoK;*bMEnUU_uW!VL%tVHj)ajXv#2^=xUkBzC{B&iwJwpUdWfq1F9(fZ+s-l5&hBdh0S?R82ZewzJaCBv4Uvy=7bie\"\", \"\"i\"\": {\"\"c\"\": \"\"encrypted_text\"\", \"\"t\"\": \"\"encrypted\"\"}, \"\"m\"\": [1339, 1758, 637, 80, 404, 1369, 1997, 1204, 586, 515, 1684, 1619, 1737, 527, 1971, 1328, 1533, 1019, 826, 1633, 1335, 1988, 1149, 609, 1915, 1448, 1775, 815, 1273, 638, 1098, 1744, 936, 445, 403, 1572, 929, 1982, 814, 234, 470, 279, 652, 1529, 2018, 512, 633, 381, 1624, 18, 1287, 1792, 1380, 1506, 1772, 1764, 145, 647, 1365, 1649, 901, 1589, 1147, 1181, 575, 1045, 1101, 176, 1150, 205, 433, 1253, 1641, 388, 1340, 1347, 894, 1333, 630, 1877, 1942, 1700, 1256, 656, 986, 1389, 1239, 1162, 369, 1761, 1606, 581, 35, 1858, 71, 2015, 1716, 1499, 1849, 1396, 113, 1336, 141, 1317, 272, 1077, 1090, 1226, 700, 358, 573], \"\"o\"\": [\"\"faa1f63cb6d36094d1aa50db6c0217eb447a987071119bb127f677b6a7ee0b4fe40eed7cd84e96e8a11bbe3ea14331f3ec4c8f149ce9d2b0253b4676c86557fcec4a5f8ca4e1ee081c66bf0a3cb594c6b5739f77f62fc5e76991869c23a97f01816cde3dfc24b2ca2fbb12b50fde324f18aa51718d681772bf9caf3c059a6748cbcaf4dd1c4fa0262d123983b784befb504aeee6567790c32f4302bd4e0b04fcecb7345d4727297a4ba4298ff9ed75a2b18ce815ded7ee6bea10738887a0f2632d42482455e4e6e2c3b6d6f4046d8ecf638aeacf644b3e00693494cd632d392a86d789c3367604821ccd1417fa52a9bc043d1e3206499cbbe04ababb7cbe27eb9d4f43849dad3af574c929891087b0ce2a54b3cddddf43e8ec7fc2f11050e5f759496653b115bbdeaffc570cf630237da8d78838f9f160abcfc2df7756fe47881687a52f6b4ec18b97f088cc62cc79b98fae646f254a9abde41a60ebe9b601a090ff977164510d3cf2a2d680ce5c80170d8b0f9ce95441d83ad7aa15a6b4732bf10fd2cbe30d1e4fff714667b161ee5ec9c5a9b7b891d107\"\", \"\"b41d89a196a35252a965ce3c330eac369ead56e9f06e2016da4d6971fe0b8d6e677e1018e7a1bd2fa0b2c1faaa12650d678352ecc81f6be879213fe78b8004b87dd7dcadec59df4dcafdb3c9aa55dcb2cc2bcf2193574b201c9a1c14764d69716f63b0c1aa30a2846696f2a1c790ca2cb26370d7e20904a8748ea98a95ee3cbb95c5f342de4e71bbbd3e45285f4862fcfe4f98116a65348afe04a9208a919b0225acf33782c1cc66313fc580e3002f7ebf4e78ec288deb49a4351355b1670bb752878c9db5c8afe7d64b43a081b06d2206a4e7e1b6a77e1e21f313e51d8518c8a256d3bd80179d413826aea416513f3c7a4a7e6b9b4cf1fa9a3c563fae6503f31985be074202bf921c2576f04469eb7524142498efb17de6e4f0b307d8c18d0a0eb08d2b3d341c33038b198894c2d367ff8f0a7c78ab04466f081b98d81130f7574824ced65e969c3c1e0dd6f55246da41b956989f64cb3b02926ad1c1bbf058fdc2e7dfeaa1bc8260852b9503cafab2b5dd860525c52368d971ca191a94b62b883d444500331e0a94b2c9df32698be8f86c7a19176f725d\"\"], \"\"s\"\": null, \"\"u\"\": \"\"962d77dfaf892b596b3255c022359e54f3e8dc8b21c3d1b32ebd05555f433192\"\", \"\"v\"\": 1, \"\"sv\"\": null, \"\"ocf\"\": null, \"\"ocv\"\": null}\")";

        // let e = parse_column(input).unwrap();

        // info!("{:?}", e);
        // assert_eq!(e);
    }

    #[test]
    pub fn data_row_to_ciphertext() {
        log::init(LogConfig::default());

        let bytes = to_message(b"D\0\0\0\x12\0\x01\0\0\0\x08{\"a\": 1}");

        let data_row = DataRow::try_from(&bytes).unwrap();
        info!("data_row {:?}", data_row);

        let s = String::from_utf8_lossy(data_row.columns[0].bytes.as_ref().unwrap().as_ref())
            .to_string();

        info!("{:?}", s);

        info!("--------------------------------");

        // let bytes = to_message(b"D\0\0\x03\x7f\0\x02\0\0\0\x08\x01\xec,\xfa\xb1\xf8\xfa \0\0\x03i\0\0\0\x01\0\0\x0e\xda\0\0\x03]\x01{\"b\": null, \"c\": \"mBbL}c9>VTROe1ai=(-kci{2FAsYPZ<&Cwi@Z%z~mIM*?YvU9j^*ZoYD@a={jBPGo=exupz&+rgENZ7s_dWd7q00*JRY-RI?9~m&n++D$ExvdVpuh\", \"i\": {\"c\": \"encrypted_jsonb\", \"t\": \"encrypted\"}, \"m\": null, \"o\": null, \"s\": null, \"u\": null, \"v\": 1, \"sv\": [{\"b\": \"8067db44a848ab32c3056a3dbe4edf16\", \"c\": \"mBbL}c9>VTROe1ai=(-kci{2FAsYPZ<&Cwi@Z%z~mIM*?YvU9j^*ZoYD@a={jBPGo=exupz&+rgENZ7s_dWd7q00*JRY-RI?9~m&n++D$ExvdVpuh\", \"m\": null, \"o\": null, \"s\": \"9493d6010fe7845d52149b697729c745\", \"u\": null, \"sv\": null, \"ocf\": null, \"ocv\": null}, {\"b\": null, \"c\": \"mBbL}c9>VTROe1ai=(-kci{2F8E5uL#Gf@OlysYJXRs3frfS{58grcR=E}q%z&+rgENZ7s_dWd7q00*JRY-RI?9~m&n++D$ExvdVpuh\", \"m\": null, \"o\": null, \"s\": \"b1f0e4bb3855bc33936ef1fddf532765\", \"u\": null, \"sv\": null, \"ocf\": null, \"ocv\": \"fbc7a11fc81f2a31c904c5b05572b054824e3b5f5ece78f1b711f93175f0a4a9726157cea247e107\"}], \"ocf\": null, \"ocv\": null}");

        let bytes = to_message(b"D\0\0\x03\x7f\0\x02\0\0\0\x08[*\xd1\x1b0C\x1e%\0\0\x03i\0\0\0\x01\0\0\x0e\xda\0\0\x03]\x01{\"b\": null, \"c\": \"mBbK>)zQdVF<ipa4|kF_+xTO|A$ps~wte&P+Z^tyEt>4nsoM`P3hAGlLxkOIJ;<B6xxK_7Z;^rl)`0JZ+d$qY=T!CumW4uYLd*mdxPpdrEHjCTpuh\", \"i\": {\"c\": \"encrypted_jsonb\", \"t\": \"encrypted\"}, \"m\": null, \"o\": null, \"s\": null, \"u\": null, \"v\": 1, \"sv\": [{\"b\": \"8067db44a848ab32c3056a3dbe4edf16\", \"c\": \"mBbK>)zQdVF<ipa4|kF_+xTO|A$ps~wte&P+Z^tyEt>4nsoM`P3hAGlLxkOIJ;<B6xxK_7Z;^rl)`0JZ+d$qY=T!CumW4uYLd*mdxPpdrEHjCTpuh\", \"m\": null, \"o\": null, \"s\": \"9493d6010fe7845d52149b697729c745\", \"u\": null, \"sv\": null, \"ocf\": null, \"ocv\": null}, {\"b\": null, \"c\": \"mBbK>)zQdVF<ipa4|kF_+xTO|8L--xD!CQNy)MaF*EF-v_W6faDXy*oU`50rZ;^rl)`0JZ+d$qY=T!CumW4uYLd*mdxPpdrEHjCTpuh\", \"m\": null, \"o\": null, \"s\": \"b1f0e4bb3855bc33936ef1fddf532765\", \"u\": null, \"sv\": null, \"ocf\": null, \"ocv\": \"fbc7a11fc81f2a31c904c5b05572b054824e3b5f5ece78f1b711f93175f0a4a9726157cea247e107\"}], \"ocf\": null, \"ocv\": null}");

        let data_row = DataRow::try_from(&bytes).unwrap();
        info!("data_row {:?}", data_row);

        let col_bytes = data_row.columns[1].bytes.as_ref().unwrap().as_ref();
        info!("data_row {:?}", col_bytes);

        let header = col_bytes[0];
        let first = col_bytes[1];

        info!(msg = "header/first", header, first);

        let s = String::from_utf8_lossy(data_row.columns[1].bytes.as_ref().unwrap().as_ref())
            .to_string();

        info!("{:?}", s);

        let value: Result<Value, _> = serde_json::from_str(&s);
        error!("{:?}", value);

        // let bytes = to_message(b"D\0\0\nc\0\x01\0\0\nY(\"{\"\"b\"\": null, \"\"c\"\": \"\"mBbK0@-;n*If_+?MK~bBli2LUCrD#`KmuT%Q)~R!_GM~3UkmfCQp@?IDPST7B(FZm^Wyh&_7F3+#30=l2}X+xdym|tf@*{dBbM0*qwWN{!!IfBCxpL`L9(u8ZewzJaCBv4Uvy=7bie\"\", \"\"i\"\": {\"\"c\"\": \"\"encrypted_text\"\", \"\"t\"\": \"\"encrypted\"\"}, \"\"m\"\": [1162, 814, 141, 1775, 1019, 145, 403, 1150, 2018, 1758, 1273, 1448, 176, 1256, 358, 1982, 1858, 1098, 573, 1589, 1340, 1389, 1533, 1877, 1764, 1506, 1700, 1499, 1317, 1226, 986, 279, 113, 445, 1090, 1737, 1761, 1915, 1684, 388, 894, 826, 586, 936, 929, 1606, 1529, 901, 637, 1149, 1333, 1181, 1744, 1849, 630, 633, 1971, 1336, 1942, 369, 1619, 1369, 512, 80, 234, 581, 1365, 381, 1101, 1328, 1641, 1997, 1204, 1649, 1716, 1772, 652, 656, 1624, 1335, 18, 1396, 35, 815, 1239, 609, 1253, 1572, 205, 1287, 575, 1077, 1988, 2015, 470, 1347, 647, 1792, 1147, 1380, 515, 71, 1633, 433, 638, 404, 700, 1045, 527, 272, 1339], \"\"o\"\": [\"\"faa1f63cb6d36094d1aa50db6c0217eb447a987071119bb127f677b6a7ee0b4fe40eed7cd84e96e8a11bbe3ea14331f3ec4c8f149ce9d2b0253b4676c86557fcec4a5f8ca4e1ee081c66bf0a3cb594c6b5739f77f62fc5e76991869c23a97f01816cde3dfc24b2ca2fbb12b50fde324f18aa51718d681772bf9caf3c059a6748cbcaf4dd1c4fa026be0637c60edc996ca201f23111e7b8b9b15f25bb0d8a56f936fe8f9881aa9d7fa64b60c15ef39749141f2c2f2ef248c71db6c581388faeb418781a1f71b3e797d74e4218989c960a14b55b1277ef949b008171c4d47792ff6c0325f6007e3bf191264a11c07fd6d9b84bf36514c253fd7eb78b2967451221673ff519bad573b16b594a0f112b533f894087046611b8e11ab819fb4cacdf43de07209af4ce0eb12a0fce536325fc7e37c0bf94a9e1533d1fc3c1fcbf148e519cb6186989785c5930da0b4aee28b0d2e0b997ad2c6b2cb717d3149a3fe8408e2078b8be1db6f3f09b4d5fc799ff09518d579a82e7cbc63796e8045113657b2a4c18521ff1dd8bca242c1f68c77ed9274812ff372b7f38d1\"\", \"\"b41d89a196a35252a965ce3c330eac369ead56e9f06e2016da4d6971fe0b8d6e677e1018e7a1bd2fa0b2c1faaa12650d678352ecc81f6be879213fe78b8004b87dd7dcadec59df4dcafdb3c9aa55dcb2cc2bcf2193574b201c9a1c14764d69716f63b0c1aa30a2846696f2a1c790ca2cb26370d7e20904a8748ea98a95ee3cbb95c5f342de4e71bbcee6326111f41e95faba5ef9065a57353a991ebd2a2822e02a92657791cd72f53eef8c771a2690ff3dc31c38e8cb5811ae377529ef76299757327c358061e6ff88bd18e440023915e02b757df9d9a20552ab18eb227fa2c3b109622a16bde8660a6829613d44aa7ee7119f8d4a52eabbc24ec862da8be60e6c6b93a22e0d79cc0a5c1d0aed4c2a557919952d3ec187ed1712e9826e4a7ab074abbc7acfd4afd9198bfb090df596d4dcec4fc33f1311a8391965c12095898c01ab3bb6554f773fc9c8b6b201be71f543b41ea8a44e0ba48b54664b383c38a20be52a0a7404943d183e04796c884d05f8bf995fc3d4443bff73a502ecdacc58fcf5b5c0d9271c87508018c675287019b1c28a6cace2feb2\"\"], \"\"s\"\": null, \"\"u\"\": \"\"962d77dfaf892b596b3255c022359e54f3e8dc8b21c3d1b32ebd05555f433192\"\", \"\"v\"\": 1, \"\"sv\"\": null, \"\"ocf\"\": null, \"\"ocv\"\": null}\")");
        // // info!("{:?}", bytes);

        // let expected = bytes.clone();

        // let data_row = DataRow::try_from(&bytes).unwrap();

        // let column = data_row.columns.first().unwrap();

        // if let Some(bytes) = &column.bytes {
        //     let e = parse_column(bytes).unwrap();
        //     info!("{:?}", e);
        // }

        // let bytes = to_message("D\0\0\x03\x7f\0\x02\0\0\0\x083?\xf2\x95\x0e\xdan(\0\0\x03i\0\0\0\x01\0\0\x0e\xda\0\0\x03]\x01{\"b\": null, \"c\": \"mBbLhoetZBO8~?NJr&Z}E4gdLA@7G^#tspnp$E<cSwV|`g?mL&)VpL4;r68JfM^`g2wB7+vjA<S8XrBDsLGvDefH<=eBrl65S7surYfjIeh>%?puh\", \"i\": {\"c\": \"encrypted_jsonb\", \"t\": \"encrypted\"}, \"m\": null, \"o\": null, \"s\": null, \"u\": null, \"v\": 1, \"sv\": [{\"b\": \"8067db44a848ab32c3056a3dbe4edf16\", \"c\": \"mBbLhoetZBO8~?NJr&Z}E4gdLA@7G^#tspnp$E<cSwV|`g?mL&)VpL4;r68JfM^`g2wB7+vjA<S8XrBDsLGvDefH<=eBrl65S7surYfjIeh>%?puh\", \"m\": null, \"o\": null, \"s\": \"9493d6010fe7845d52149b697729c745\", \"u\": null, \"sv\": null, \"ocf\": null, \"ocv\": null}, {\"b\": null, \"c\": \"mBbLhoetZBO8~?NJr&Z}E4gdL8TC^?h;G0jOkZzwkAM?Fxh`fQzSi3BM%Kh2vjA<S8XrBDsLGvDefH<=eBrl65S7surYfjIeh>%?puh\", \"m\": null, \"o\": null, \"s\": \"b1f0e4bb3855bc33936ef1fddf532765\", \"u\": null, \"sv\": null, \"ocf\": null, \"ocv\": \"fbc7a11fc81f2a31c904c5b05572b054824e3b5f5ece78f1b711f93175f0a4a9726157cea247e107\"}], \"ocf\": null, \"ocv\": null}");

        // let e = parse_column(input).unwrap();

        // for col in data_row.columns {
        //     if let Some(b) = col.bytes {
        //         let mut s = String::from_utf8_lossy(&b);
        //         info!("{:?}", s);
        //         let r = parse_encrypted(&mut &*s);
        //         info!("{:?}", r);
        //     }
        // }
    }

    #[test]
    pub fn parse_data_row() {
        let bytes = to_message(b"D\0\0\0\x0e\0\x01\0\0\0\x04\0\0\x1e\xa2");
        let expected = bytes.clone();

        let data_row = DataRow::try_from(&bytes).unwrap();

        let data_col = data_row.columns.first().unwrap();

        let mut buf: &[u8] = data_col.bytes.as_ref().unwrap();
        let value = buf.get_i32();
        assert_eq!(value, 7842);

        let bytes = BytesMut::try_from(data_row).unwrap();
        assert_eq!(bytes, expected);
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
