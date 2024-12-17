use super::{BackendCode, NULL};
use crate::error::{Error, ProtocolError};
use bytes::{Buf, BufMut, Bytes, BytesMut};
use std::io::Cursor;

#[derive(Debug, Clone)]
pub struct DataRow {
    pub columns: Vec<DataColumn>,
}

#[derive(Debug, Clone)]
pub struct DataColumn {
    pub data: Option<Bytes>,
}

impl DataRow {
    fn len_of_columns(&self) -> usize {
        let column_len_size = size_of::<i32>(); // len of column len

        self.columns
            .iter()
            .map(|col| column_len_size + col.data.as_ref().map(|b| b.len()).unwrap_or(0))
            .sum()
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
                columns.push(DataColumn { data: None });
            } else {
                let data = cursor.copy_to_bytes(len as usize);
                columns.push(DataColumn { data: Some(data) });
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

        if let Some(data) = data_column.data {
            bytes.put_i32(data.len() as i32);
            bytes.put_slice(&data);
        } else {
            bytes.put_i32(NULL);
        }

        Ok(bytes)
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
        let column = DataColumn { data: None };
        let data_row = DataRow {
            columns: vec![column],
        };
        assert_eq!(data_row.len_of_columns(), 4);

        let data = Bytes::from("");
        let column = DataColumn { data: Some(data) };
        let data_row = DataRow {
            columns: vec![column],
        };
        assert_eq!(data_row.len_of_columns(), 4);

        let data = Bytes::from("blah");
        let column = DataColumn { data: Some(data) };
        let data_row = DataRow {
            columns: vec![column],
        };
        assert_eq!(data_row.len_of_columns(), 8);

        let mut columns = Vec::new();
        for _ in 1..5 {
            let data = Bytes::from("blah");
            let column = DataColumn { data: Some(data) };
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

        let mut buf: &[u8] = data_col.data.as_ref().unwrap();
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

        let buf: &[u8] = data_col.data.as_ref().unwrap();
        let value = String::from_utf8_lossy(buf).to_string();
        assert_eq!(value, "blahvtha");
    }

    #[test]
    pub fn parse_data_row_with_null_column() {
        let bytes = to_message(b"D\0\0\0\n\0\x01\xff\xff\xff\xff");

        let data_row = DataRow::try_from(&bytes).expect("ok");

        let data_col = data_row.columns.first().unwrap();

        assert_eq!(data_col.data, None);
    }
}
