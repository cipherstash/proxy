use std::{ffi::CString, io::Cursor};

use bytes::{Buf, BufMut, BytesMut};
use tracing::info;

use crate::{
    error::{Error, ProtocolError},
    postgresql::{format_code::FormatCode, protocol::BytesMutReadString},
};

use super::BackendCode;

#[derive(Debug)]
pub struct RowDescription {
    pub fields: Vec<RowDescriptionField>,
}

#[derive(Debug)]
pub struct RowDescriptionField {
    pub name: String,
    pub table_oid: i32,
    pub table_column: i16,
    pub type_oid: postgres_types::Type,
    pub type_size: i16,
    pub type_modifier: i32,
    pub format_code: FormatCode,
    dirty: bool,
}

impl RowDescription {
    pub fn should_rewrite(&self) -> bool {
        self.fields.iter().any(|f| f.should_rewrite())
    }
}

impl RowDescriptionField {
    pub fn rewrite_type_oid(&mut self, type_oid: postgres_types::Type) {
        self.type_oid = type_oid;
        self.dirty = true;
    }

    pub fn should_rewrite(&self) -> bool {
        self.dirty
    }
}

impl TryFrom<&BytesMut> for RowDescription {
    type Error = Error;

    fn try_from(bytes: &BytesMut) -> Result<RowDescription, Error> {
        let mut cursor = Cursor::new(bytes);

        let code = cursor.get_u8();

        if BackendCode::from(code) != BackendCode::RowDescription {
            return Err(ProtocolError::UnexpectedMessageCode {
                expected: BackendCode::RowDescription.into(),
                received: code as char,
            }
            .into());
        }

        let _len = cursor.get_i32(); // move the cursor
        let field_count = cursor.get_i16() as usize;

        let bytes = cursor.copy_to_bytes(cursor.remaining()).into();

        let fields = std::iter::repeat_with(|| RowDescriptionField::try_from(&bytes))
            .take(field_count)
            .collect::<Result<_, _>>()?;

        Ok(RowDescription { fields })
    }
}

impl TryFrom<RowDescription> for BytesMut {
    type Error = Error;

    fn try_from(row_description: RowDescription) -> Result<BytesMut, Error> {
        let mut bytes = BytesMut::new();

        let fields = row_description
            .fields
            .into_iter()
            .map(BytesMut::try_from)
            .collect::<Result<Vec<_>, _>>()?;

        let field_count = fields.len();
        let field_size = fields.iter().map(|x| x.len()).sum::<usize>();

        let len = size_of::<i32>() + size_of::<i16>() + field_size;

        bytes.put_u8(BackendCode::RowDescription.into());
        bytes.put_i32(len as i32);
        bytes.put_i16(field_count as i16);

        for field in fields.into_iter() {
            bytes.put_slice(&field);
        }

        Ok(bytes)
    }
}

impl TryFrom<&BytesMut> for RowDescriptionField {
    type Error = Error;

    fn try_from(bytes: &BytesMut) -> Result<RowDescriptionField, Self::Error> {
        let mut cursor = Cursor::new(bytes);

        let name = cursor.read_string()?;

        let table_oid = cursor.get_i32();
        let table_column = cursor.get_i16();
        let type_oid = cursor.get_i32();

        let type_oid = postgres_types::Type::from_oid(type_oid as u32)
            .unwrap_or(postgres_types::Type::UNKNOWN);

        let type_size = cursor.get_i16();
        let type_modifier = cursor.get_i32();
        let format_code = cursor.get_i16().into();

        Ok(Self {
            name,
            table_oid,
            table_column,
            type_oid,
            type_size,
            type_modifier,
            format_code,
            dirty: false,
        })
    }
}

impl TryFrom<RowDescriptionField> for BytesMut {
    type Error = Error;

    fn try_from(field: RowDescriptionField) -> Result<Self, Self::Error> {
        let mut bytes = BytesMut::new();

        let name = CString::new(field.name)?;
        let name = name.as_bytes_with_nul();

        bytes.put_slice(name);
        bytes.put_i32(field.table_oid);
        bytes.put_i16(field.table_column);
        bytes.put_i32(field.type_oid.oid() as i32);
        bytes.put_i16(field.type_size);
        bytes.put_i32(field.type_modifier);
        bytes.put_i16(field.format_code.into());

        Ok(bytes)
    }
}

#[cfg(test)]
mod tests {

    use bytes::BytesMut;
    use tracing::info;

    use crate::{log, postgresql::messages::row_description::RowDescription};

    fn to_message(s: &[u8]) -> BytesMut {
        BytesMut::from(s)
    }

    #[test]
    pub fn parse_row_description() {
        log::init();
        let bytes = to_message(
            b"T\0\0\0!\0\x01TimeZone\0\0\0\0\0\0\0\0\0\0\x19\xff\xff\xff\xff\xff\xff\0\0",
        );

        let expected = bytes.clone();

        let row_description = RowDescription::try_from(&bytes).expect("ok");

        info!("{:?}", row_description);

        assert_eq!(row_description.fields.len(), 1);
        assert_eq!(row_description.fields[0].name, "TimeZone");

        let bytes = BytesMut::try_from(row_description).expect("ok");
        assert_eq!(bytes, expected);
    }
}
