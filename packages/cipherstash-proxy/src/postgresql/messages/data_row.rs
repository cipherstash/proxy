use super::{maybe_json, maybe_jsonb, BackendCode, NULL};
use crate::{
    eql,
    error::{Error, ProtocolError},
};
use bytes::{Buf, BufMut, Bytes, BytesMut};
use std::io::Cursor;
use tracing::{debug, info, warn};

#[derive(Debug, Clone)]
pub struct DataRow {
    pub columns: Vec<DataColumn>,
}

#[derive(Debug, Clone)]
pub struct DataColumn {
    bytes: Option<BytesMut>,
}

impl DataRow {
    pub fn to_ciphertext(&self) -> Result<Vec<Option<eql::Ciphertext>>, Error> {
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

    pub fn update_from_ciphertext(
        &mut self,
        plaintexts: &[Option<eql::Plaintext>],
    ) -> Result<(), Error> {
        for (idx, pt) in plaintexts.iter().enumerate() {
            if let Some(pt) = pt {
                let bytes = pt.plaintext.as_bytes().to_vec();
                self.columns[idx].rewrite(&bytes);
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
            .map_or(false, |b| maybe_jsonb(&b) || maybe_json(&b))
    }

    pub fn rewrite(&mut self, b: &[u8]) {
        if let Some(ref mut bytes) = self.bytes {
            bytes.clear();

            bytes.extend_from_slice(b);
        }
        // self.dirty = true;
    }

    ///
    /// If the json format looks binary, returns a reference to the bytes without the jsonb header byte
    ///
    pub fn json_bytes(&self) -> Option<&[u8]> {
        self.bytes.as_ref().map_or(None, |b| {
            if maybe_jsonb(&b) {
                Some(&b[1..])
            } else if maybe_json(&b) {
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
                // let data = cursor.copy_to_bytes(len as usize);
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

impl From<&DataColumn> for Option<eql::Ciphertext> {
    fn from(col: &DataColumn) -> Self {
        match col.json_bytes() {
            Some(bytes) => match serde_json::from_slice(bytes) {
                Ok(ct) => Some(ct),
                Err(e) => {
                    debug!(error = e.to_string(), "Failed to parse parameter");
                    None
                }
            },
            None => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::postgresql::messages::data_row::DataColumn;

    use super::DataRow;
    use bytes::{Buf, Bytes, BytesMut};

    fn to_message(s: &[u8]) -> BytesMut {
        BytesMut::from(s)
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

    #[test]
    pub fn parse_data_row() {
        let bytes = to_message(b"D\0\0\0\x0e\0\x01\0\0\0\x04\0\0\x1e\xa2");
        let expected = bytes.clone();

        let data_row = DataRow::try_from(&bytes).expect("ok");

        let data_col = data_row.columns.first().unwrap();

        let mut buf: &[u8] = data_col.bytes.as_ref().unwrap();
        let value = buf.get_i32();
        assert_eq!(value, 7842);

        let bytes = BytesMut::try_from(data_row).expect("ok");
        assert_eq!(bytes, expected);
    }

    #[test]
    pub fn parse_data_row_with_columns() {
        let bytes = to_message(
            b"D\0\0\09\0\x03\0\0\0\x08blahvtha\0\0\0\x0242\0\0\0\x1d2023-12-16 01:52:25.031985+00",
        );

        let data_row = DataRow::try_from(&bytes).expect("ok");

        let data_col = data_row.columns.first().unwrap();

        let buf: &[u8] = data_col.bytes.as_ref().unwrap();
        let value = String::from_utf8_lossy(buf).to_string();
        assert_eq!(value, "blahvtha");
    }

    #[test]
    pub fn parse_data_row_with_null_column() {
        let bytes = to_message(b"D\0\0\0\n\0\x01\xff\xff\xff\xff");

        let data_row = DataRow::try_from(&bytes).expect("ok");

        let data_col = data_row.columns.first().unwrap();

        assert_eq!(data_col.bytes, None);
    }
}
