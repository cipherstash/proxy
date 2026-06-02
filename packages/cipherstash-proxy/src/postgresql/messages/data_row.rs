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
    use super::{DataRow, NULL};
    use crate::Identifier;
    use crate::{
        config::{LogConfig, LogLevel},
        log,
        postgresql::{messages::data_row::DataColumn, Column},
    };
    use bytes::{BufMut, BytesMut};
    use cipherstash_client::eql::{
        EncryptedPayload, EqlCiphertext, SteVecEntry, SteVecEntryTerm, SteVecPayload,
        EQL_SCHEMA_VERSION,
    };
    use cipherstash_client::schema::{ColumnConfig, ColumnType};
    use cipherstash_client::zerokms::EncryptedRecord;

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

    /// A dummy `EncryptedRecord` for the `c` ciphertext field. The data_row
    /// tests only exercise wire parsing and JSON deserialization, never
    /// decryption, so the record does not need to be cryptographically valid.
    fn encrypted_record() -> EncryptedRecord {
        EncryptedRecord {
            iv: Default::default(),
            ciphertext: vec![1; 16],
            tag: vec![2; 16],
            descriptor: "encrypted/column".to_string(),
            keyset_id: None,
            decryption_policy: None,
        }
    }

    /// Builds an EQL v2.3 scalar (`k = "ct"`) storage payload as a JSON string.
    fn scalar_ciphertext_json(column: &str) -> String {
        let ciphertext = EqlCiphertext::Encrypted(EncryptedPayload {
            version: EQL_SCHEMA_VERSION,
            identifier: Identifier::new("encrypted", column),
            ciphertext: encrypted_record(),
            hmac_256: Some("deadbeef".into()),
            bloom_filter: None,
            ore_block_u64_8_256: None,
        });
        serde_json::to_string(&ciphertext).unwrap()
    }

    /// Builds an EQL v2.3 STE-vector (`k = "sv"`) storage payload as a JSON string.
    fn ste_vec_ciphertext_json(column: &str) -> String {
        let ciphertext = EqlCiphertext::SteVec(SteVecPayload {
            version: EQL_SCHEMA_VERSION,
            identifier: Identifier::new("encrypted", column),
            ste_vec: vec![SteVecEntry {
                selector: "9493d6010fe7845d52149b697729c745".to_string(),
                ciphertext: encrypted_record(),
                is_array: None,
                term: SteVecEntryTerm::Hmac {
                    hmac_256: "deadbeef".into(),
                },
            }],
        });
        serde_json::to_string(&ciphertext).unwrap()
    }

    /// Frames a `DataRow` backend message from raw column payloads
    /// (`None` for a SQL NULL column).
    fn data_row_message(columns: &[Option<&[u8]>]) -> BytesMut {
        let mut body = BytesMut::new();
        body.put_i16(columns.len() as i16);
        for column in columns {
            match column {
                Some(data) => {
                    body.put_i32(data.len() as i32);
                    body.put_slice(data);
                }
                None => body.put_i32(NULL),
            }
        }

        let mut message = BytesMut::new();
        message.put_u8(b'D');
        message.put_i32((size_of::<i32>() + body.len()) as i32);
        message.put_slice(&body);
        message
    }

    /// Encodes a JSON payload as a PostgreSQL text-format composite value:
    /// `("...")` with embedded double quotes doubled.
    fn text_encrypted_column(json: &str) -> Vec<u8> {
        format!("(\"{}\")", json.replace('"', "\"\"")).into_bytes()
    }

    /// Encodes a JSON payload as a PostgreSQL binary-format value: a
    /// single-field record header followed by a `jsonb` value.
    fn binary_encrypted_column(json: &str) -> Vec<u8> {
        let mut bytes = BytesMut::new();
        bytes.put_i32(1); // number of fields
        bytes.put_i32(3802); // jsonb type OID
        bytes.put_i32((json.len() + 1) as i32); // field length incl. jsonb version byte
        bytes.put_u8(1); // jsonb version
        bytes.put_slice(json.as_bytes());
        bytes.to_vec()
    }

    #[test]
    pub fn to_ciphertext_with_binary_encoding() {
        log::init(LogConfig::with_level(LogLevel::Debug));

        //  Binary
        // SELECT id, encrypted_text FROM encrypted WHERE id = $1
        let json = scalar_ciphertext_json("encrypted_text");
        let id = 1234_i64.to_be_bytes();
        let encrypted_text = binary_encrypted_column(&json);
        let bytes = data_row_message(&[Some(id.as_slice()), Some(encrypted_text.as_slice())]);
        let mut data_row = DataRow::try_from(&bytes).unwrap();

        let column_config = column_config_with_id("encrypted_text");
        let encrypted = data_row.as_ciphertext(&column_config);

        assert_eq!(encrypted.len(), 2);

        // Two rows
        assert!(encrypted[0].is_none());
        assert!(encrypted[1].is_some());

        assert_eq!(
            column_config[1].as_ref().unwrap().identifier,
            *encrypted[1].as_ref().unwrap().identifier()
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
        let json = ste_vec_ciphertext_json("encrypted_jsonb");
        let encrypted_jsonb = text_encrypted_column(&json);
        let bytes = data_row_message(&[Some(encrypted_jsonb.as_slice())]);
        let mut data_row = DataRow::try_from(&bytes).unwrap();

        assert!(data_row.columns[0].bytes.is_some());

        let column_config = vec![column_config("encrypted_jsonb")];
        let encrypted = data_row.as_ciphertext(&column_config);

        assert_eq!(encrypted.len(), 1);
        assert!(encrypted[0].is_some());

        assert_eq!(
            column_config[0].as_ref().unwrap().identifier,
            *encrypted[0].as_ref().unwrap().identifier()
        );
    }

    #[test]
    pub fn to_ciphertext_with_text_encoding_and_null() {
        log::init(LogConfig::with_level(LogLevel::Debug));

        // SELECT * FROM encrypted WHERE id = $1;
        // Only encrypted_text is NOT NULL
        let json = scalar_ciphertext_json("encrypted_text");
        let encrypted_text = text_encrypted_column(&json);
        let bytes = data_row_message(&[
            Some(b"1297231342".as_slice()),
            None,
            Some(encrypted_text.as_slice()),
            None,
            None,
            None,
            None,
            None,
            None,
            None,
        ]);

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
            *encrypted[2].as_ref().unwrap().identifier()
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
