use super::{maybe_json, maybe_jsonb, BackendCode, NULL};
use crate::{
    eql,
    error::{Error, ProtocolError},
    log::MAPPER,
};
use bytes::{Buf, BufMut, BytesMut};
use std::io::Cursor;
use tracing::debug;

#[derive(Debug, Clone)]
pub struct DataRow {
    pub columns: Vec<DataColumn>,
}

#[derive(Debug, Clone)]
pub struct DataColumn {
    bytes: Option<BytesMut>,
}

impl DataRow {
    pub fn to_ciphertext(&self) -> Result<Vec<Option<eql::EqlEncrypted>>, Error> {
        Ok(self.columns.iter().map(|col| col.into()).collect())
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

    pub fn maybe_ciphertext(&self) -> bool {
        self.bytes
            .as_ref()
            .is_some_and(|b| maybe_jsonb(b) || maybe_json(b))
    }

    pub fn rewrite(&mut self, b: &[u8]) {
        if let Some(ref mut bytes) = self.bytes {
            bytes.clear();
            bytes.extend_from_slice(b);
        }
    }

    ///
    /// If the json format looks binary, returns a reference to the bytes without the jsonb header byte
    ///
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

impl From<&DataColumn> for Option<eql::EqlEncrypted> {
    fn from(col: &DataColumn) -> Self {
        debug!(target: MAPPER, data_column = ?col);
        match col.json_bytes() {
            Some(bytes) => match serde_json::from_slice(bytes) {
                Ok(ct) => Some(ct),
                Err(err) => {
                    debug!(target: MAPPER, msg = "Could not convert DataColumn to Ciphertext", error = err.to_string());
                    None
                }
            },
            None => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::DataRow;
    use crate::{config::LogConfig, log, postgresql::messages::data_row::DataColumn};
    use bytes::{Buf, BytesMut};
    use cipherstash_client::zerokms::EncryptedRecord;
    use recipher::key::Iv;
    use tracing::info;
    use uuid::Uuid;

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
    pub fn data_row_to_ciphertext() {
        log::init(LogConfig::default());

        let record = record();

        let s = record.to_mp_base85().unwrap();
        info!("{:?}", s);

        // "{\"c\": \"mBbKx=EbyVyx>mNt9E<k5A&(S8?o+de4F^|i^}7e3l4YE2r(f|W`0Und}s4|#2_A>;-h3xf8wDrq~v|IvQ=jXYG!u4Uu9SI)@Q+xmSd+PWo=<;Y$Ct\",\"k\": \"ct\",\"i\": {\"t\": \"\"users\"\",\"c\": \"\"email\"\"},\"v\": 1}";

        let bytes = to_message(b"D\0\0\0i\0\x01\0\0\0_\x01{\"c\": \"mBbKx=EbyVyx>mNt9E<k5A&(S8?o+de4F^|i^}7e3l4YE2r(f|W`0Und}s4|#2_A>;-h3xf8wDrq~v|IvQ=jXYG!u4Uu9SI)@Q+xmSd+PWo=<;Y$Ct\",\"k\": \"ct\",\"i\": {\"t\": \"\"users\"\",\"c\": \"\"email\"\"},\"v\": 1}");
        // let expected = bytes.clone();

        let _data_row = DataRow::try_from(&bytes).unwrap();

        // let ciphertext = data_row.to_ciphertext().expect("ok");

        // let ct = ciphertext.first();
        // assert!(ct.is_some());

        // info!("{:?}", ciphertext.first());

        // let column = ciphertext.first().unwrap().as_ref().unwrap();

        // assert_eq!(column.kind, "ct");
    }

    // #[test]
    // pub fn data_column_to_json_bytes() {
    //     log::init(&None);
    //     let bytes = to_message(b"D\0\0\0i\0\x01\0\0\0_\x01{\"c\": \"51b72947dc25481880175ef53a35af34\", \"i\": {\"c\": \"name\", \"t\": \"users\"}, \"k\": \"ct\", \"v\": 1}");
    //     // let expected = bytes.clone();

    //     let data_row = DataRow::try_from(&bytes).expect("ok");

    //     let ciphertext = data_row.to_ciphertext().expect("ok");

    //     info!("{:?}", data_row);

    //     // info!("{:?}", ciphertext.first());

    //     // let column = ciphertext.first().unwrap().as_ref().unwrap();

    //     // assert_eq!(column.kind, "ct");
    // }

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
