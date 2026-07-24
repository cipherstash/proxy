use super::{BackendCode, NULL};
use crate::EqlCiphertext;
use crate::{
    error::{EncryptError, Error, ProtocolError},
    log::DECRYPT,
    postgresql::Column,
};
use bytes::{Buf, BufMut, BytesMut};
use std::io::Cursor;
use tracing::{debug, error};

/// Leading byte of `jsonb`'s binary wire format. PostgreSQL has only ever
/// emitted version 1.
const JSONB_BINARY_VERSION: u8 = 1;

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
                        .to_eql_ciphertext()
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

impl DataColumn {
    /// Parse this column's bytes into an [`EqlCiphertext`].
    ///
    /// EQL v3 column types (`eql_v3_text_eq`, `eql_v3_integer_ord`, …) are
    /// DOMAINS over `jsonb`, so a value arrives with jsonb's representation.
    ///
    /// EQL v2's `eql_v2_encrypted` was a composite type, which is why this
    /// used to strip a `("…")` wrapper in text and a 12-byte rowtype header
    /// in binary. Neither exists any more — a domain is wire-identical to its
    /// base type.
    ///
    ///   text   — the JSON object itself, no wrapper and no doubled quotes
    ///   binary — a 1-byte jsonb version header followed by the JSON text
    ///
    /// The two are told apart by the leading byte: the version header is
    /// `0x01`, and JSON text for an EQL payload always starts with `{`.
    ///
    /// The JSON is usually a self-describing payload — a scalar `{v,i,c,…}` or
    /// a SteVec document `{v,k:"sv",i,h,sv}` — and deserialises directly. The
    /// exception is a JSON field access (`eql_v3."->"(…)` /
    /// `eql_v3.jsonb_path_query(…)`), whose result is a single
    /// `eql_v3_json_entry` (`{v,i,h,s,c,op}`) — one SteVec entry merged with
    /// its document envelope. That has a `c`, so it would masquerade as a
    /// scalar `Encrypted` payload, but its `c` is an *entry* ciphertext that
    /// only decrypts with the entry's selector-derived nonce. So when the
    /// payload is a bare entry (see [`is_json_entry`]) it is reshaped into a
    /// one-entry SteVec document (see [`json_entry_into_ste_vec_document`]) and
    /// the ordinary SteVec decrypt path recovers the field value.
    fn to_eql_ciphertext(&self) -> Result<EqlCiphertext, Error> {
        let Some(bytes) = &self.bytes else {
            return Err(EncryptError::ColumnCouldNotBeParsed.into());
        };

        let json = match bytes.first() {
            Some(&JSONB_BINARY_VERSION) => &bytes[1..],
            Some(_) => &bytes[..],
            None => return Err(EncryptError::ColumnCouldNotBeParsed.into()),
        };

        let mut value: serde_json::Value =
            serde_json::from_slice(json).map_err(log_deserialise_error)?;

        if is_json_entry(&value) {
            json_entry_into_ste_vec_document(&mut value)?;
        }

        serde_json::from_value(value).map_err(log_deserialise_error)
    }
}

/// Whether a decoded EQL payload is a bare `eql_v3_json_entry` — the result of
/// a JSON field access (`eql_v3."->"(…)` / `eql_v3.jsonb_path_query(…)`).
///
/// A root-level selector `s` is the tell: a scalar `Encrypted` payload has no
/// selector at all, and a SteVec document carries selectors only inside its
/// `sv[]` entries, never at the root.
fn is_json_entry(value: &serde_json::Value) -> bool {
    value.get("s").is_some()
}

/// Reshape a single `eql_v3_json_entry` into a one-entry SteVec document.
///
/// The entry is `{v,i,h,s,c,op}`: document-envelope fields (`v`, `i`, `h`)
/// alongside one SteVec entry's fields (`s`, `c`, the optional array marker
/// `a`, and the optional ordering term `op`). Move the entry fields under
/// `sv:[{…}]` and tag the object as a SteVec (`k:"sv"`), yielding
/// `{v,k:"sv",i,h,sv:[{s,c,a?,op?}]}` — the shape an [`EqlCiphertext`] SteVec
/// document deserialises from and the decrypt path knows how to open.
fn json_entry_into_ste_vec_document(value: &mut serde_json::Value) -> Result<(), Error> {
    use serde_json::Value;

    let object = value
        .as_object_mut()
        .ok_or(EncryptError::ColumnCouldNotBeParsed)?;

    let mut entry = serde_json::Map::new();
    for key in ["s", "c", "a", "op"] {
        if let Some(field) = object.remove(key) {
            entry.insert(key.to_owned(), field);
        }
    }

    object.insert("k".to_owned(), Value::String("sv".to_owned()));
    object.insert("sv".to_owned(), Value::Array(vec![Value::Object(entry)]));

    Ok(())
}

fn log_deserialise_error(err: serde_json::Error) -> Error {
    debug!(target: DECRYPT, error = err.to_string());
    err.into()
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

    // The four `to_ciphertext_*` fixtures below are REAL EQL v3 wire captures
    // taken from Postgres -> Proxy `DataRow` messages for the `encrypted` test
    // table (regenerated via a live encrypt round-trip against ZeroKMS + EQL
    // v3.0.2). They exercise `DataRow::try_from` + `as_ciphertext` across the
    // binary (jsonb `0x01` version header) and text (bare JSON) wire encodings,
    // and NULL columns.
    #[test]
    pub fn to_ciphertext_with_binary_encoding() {
        log::init(LogConfig::with_level(LogLevel::Debug));

        // `SELECT encrypted_text FROM encrypted WHERE id = $1` (extended/binary):
        // the jsonb column arrives as `0x01` + the v3 EqlCiphertextV3 JSON.
        let bytes = to_message(b"D\x00\x00\x03\x16\x00\x01\x00\x00\x03\x0c\x01{\"c\": \"mBbL3gJuL?E})+>NeOq5<7N279rs9aRhBwjz3>wOdg{d64myql`6cXIurM_?B|pR<+M8(SeOLoLt~axenSv%=hCOb&m`FC5F;fS-ykq76u4Qgxa(QrcWn^D;Wq5SN5EJ90LtnW_NroxKJj=JLK>\", \"i\": {\"c\": \"encrypted_text\", \"t\": \"encrypted\"}, \"v\": 3, \"bf\": [1512, 1681, 836, 288, 1837, 1131, 415, 1430, 60, 812, 1990, 1211, 1368, 343, 1473, 1980, 598, 1549, 457, 1389, 1557, 941, 494, 1009, 1604, 1033, 2046, 222, 2012, 671, 7, 1525, 265, 901, 743, 543, 1771, 1149, 890, 755, 1974, 1960, 387, 1947, 1298, 130, 1758, 1060, 268, 844, 1375, 746, 1251, 2040], \"hm\": \"96aeaf9852416229d6b33ceb018d9abc90d70cbe7632539d69ef1462c9aa86a0\", \"op\": \"00bf0281ccb68cc6fe496bb1c8277e3484f6392517d5b8425536af7ec00ad7cc40e17e6336568ac4ed98dd659f7581f8a113fe5669b89833d9dd8eadc587a8950b6bd94f872e7f4205a6859e071df47134d3cccf1e53295417\"}");
        let mut data_row = DataRow::try_from(&bytes).unwrap();

        let column_config = vec![column_config("encrypted_text")];
        let encrypted = data_row.as_ciphertext(&column_config);

        assert_eq!(encrypted.len(), 1);
        assert!(encrypted[0].is_some());
        assert_eq!(
            &column_config[0].as_ref().unwrap().identifier,
            encrypted[0].as_ref().unwrap().identifier()
        );
    }

    #[test]
    pub fn to_ciphertext_with_binary_encoding_and_null() {
        log::init(LogConfig::with_level(LogLevel::Debug));

        // `SELECT encrypted_text, encrypted_bool FROM encrypted WHERE id = $1`
        // (binary), encrypted_text set, encrypted_bool NULL.
        let bytes = to_message(b"D\x00\x00\x03\x1a\x00\x02\x00\x00\x03\x0c\x01{\"c\": \"mBbL3gJuL?E})+>NeOq5<7N279rs9aRhBwjz3>wOdg{d64myql`6cXIurM_?B|pR<+M8(SeOLoLt~axenSv%=hCOb&m`FC5F;fS-ykq76u4Qgxa(QrcWn^D;Wq5SN5EJ90LtnW_NroxKJj=JLK>\", \"i\": {\"c\": \"encrypted_text\", \"t\": \"encrypted\"}, \"v\": 3, \"bf\": [1512, 1681, 836, 288, 1837, 1131, 415, 1430, 60, 812, 1990, 1211, 1368, 343, 1473, 1980, 598, 1549, 457, 1389, 1557, 941, 494, 1009, 1604, 1033, 2046, 222, 2012, 671, 7, 1525, 265, 901, 743, 543, 1771, 1149, 890, 755, 1974, 1960, 387, 1947, 1298, 130, 1758, 1060, 268, 844, 1375, 746, 1251, 2040], \"hm\": \"96aeaf9852416229d6b33ceb018d9abc90d70cbe7632539d69ef1462c9aa86a0\", \"op\": \"00bf0281ccb68cc6fe496bb1c8277e3484f6392517d5b8425536af7ec00ad7cc40e17e6336568ac4ed98dd659f7581f8a113fe5669b89833d9dd8eadc587a8950b6bd94f872e7f4205a6859e071df47134d3cccf1e53295417\"}\xff\xff\xff\xff");
        let mut data_row = DataRow::try_from(&bytes).unwrap();

        let column_config = vec![
            column_config("encrypted_text"),
            column_config("encrypted_bool"),
        ];
        let encrypted = data_row.as_ciphertext(&column_config);

        assert_eq!(encrypted.len(), 2);
        assert!(encrypted[0].is_some());
        assert!(encrypted[1].is_none());
    }

    #[test]
    pub fn to_ciphertext_with_text_encoding() {
        log::init(LogConfig::with_level(LogLevel::Debug));

        // `SELECT encrypted_jsonb FROM encrypted WHERE id = 2` (simple/text): the
        // jsonb column arrives as bare JSON text, no version header.
        let bytes = to_message(b"D\x00\x00\x027\x00\x01\x00\x00\x02-{\"h\": \"l*AC8+7wO)sD**%APm>F3Bc9FAg#FNCmyISKh%bW{NbL}o`gZpBwFD}ye0IoZJ}<8La$|RV{&<LbY)~;YIARHV#E*=<D)}gxkyQdDaAa?x2iz\", \"i\": {\"c\": \"encrypted_jsonb\", \"t\": \"encrypted\"}, \"k\": \"sv\", \"v\": 3, \"sv\": [{\"c\": \"=)|^uOwqW#H)TqK3PNbj|0;X%JkdzdG4-n\", \"s\": \"4aea36922168767cc743f65936aca693\"}, {\"c\": \"x%zSLSuK0+1GBi+xNdO9%dFT^^Z\", \"s\": \"956c1af474fb873d521afac3f1fed11e\", \"op\": \"00edcfafe10ba38a5a106d2f12d2f7f57238\"}, {\"c\": \"b#r<7aRQs|X-ca_T!nIL<t}C\", \"s\": \"86bc88ee9ebbf7a7bdf1ca2f5289b175\"}, {\"c\": \"`v9`QIuYF_El2G2gz+I}vEv8\", \"s\": \"38e70163b339d6b3bb126618a630d624\"}]}");
        let mut data_row = DataRow::try_from(&bytes).unwrap();

        let column_config = vec![column_config("encrypted_jsonb")];
        let encrypted = data_row.as_ciphertext(&column_config);

        assert_eq!(encrypted.len(), 1);
        assert!(encrypted[0].is_some());
        assert_eq!(
            &column_config[0].as_ref().unwrap().identifier,
            encrypted[0].as_ref().unwrap().identifier()
        );
    }

    #[test]
    pub fn to_ciphertext_with_text_encoding_and_null() {
        log::init(LogConfig::with_level(LogLevel::Debug));

        // `SELECT encrypted_text, encrypted_bool FROM encrypted WHERE id = 1`
        // (text), encrypted_text set, encrypted_bool NULL.
        let bytes = to_message(b"D\x00\x00\x03\x19\x00\x02\x00\x00\x03\x0b{\"c\": \"mBbL3gJuL?E})+>NeOq5<7N279rs9aRhBwjz3>wOdg{d64myql`6cXIurM_?B|pR<+M8(SeOLoLt~axenSv%=hCOb&m`FC5F;fS-ykq76u4Qgxa(QrcWn^D;Wq5SN5EJ90LtnW_NroxKJj=JLK>\", \"i\": {\"c\": \"encrypted_text\", \"t\": \"encrypted\"}, \"v\": 3, \"bf\": [1512, 1681, 836, 288, 1837, 1131, 415, 1430, 60, 812, 1990, 1211, 1368, 343, 1473, 1980, 598, 1549, 457, 1389, 1557, 941, 494, 1009, 1604, 1033, 2046, 222, 2012, 671, 7, 1525, 265, 901, 743, 543, 1771, 1149, 890, 755, 1974, 1960, 387, 1947, 1298, 130, 1758, 1060, 268, 844, 1375, 746, 1251, 2040], \"hm\": \"96aeaf9852416229d6b33ceb018d9abc90d70cbe7632539d69ef1462c9aa86a0\", \"op\": \"00bf0281ccb68cc6fe496bb1c8277e3484f6392517d5b8425536af7ec00ad7cc40e17e6336568ac4ed98dd659f7581f8a113fe5669b89833d9dd8eadc587a8950b6bd94f872e7f4205a6859e071df47134d3cccf1e53295417\"}\xff\xff\xff\xff");
        let mut data_row = DataRow::try_from(&bytes).unwrap();

        let column_config = vec![
            column_config("encrypted_text"),
            column_config("encrypted_bool"),
        ];
        let encrypted = data_row.as_ciphertext(&column_config);

        assert_eq!(encrypted.len(), 2);
        assert!(encrypted[0].is_some());
        assert!(encrypted[1].is_none());
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
