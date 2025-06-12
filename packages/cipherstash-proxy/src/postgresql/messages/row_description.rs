use std::{ffi::CString, io::Cursor};

use bytes::{Buf, BufMut, BytesMut};
use postgres_types::Type;

use crate::{
    error::{Error, ProtocolError},
    postgresql::{format_code::FormatCode, protocol::BytesMutReadString},
    SIZE_I16, SIZE_I32,
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
    pub type_oid: i32,
    pub type_size: i16,
    pub type_modifier: i32,
    pub format_code: FormatCode,
    dirty: bool,
}

impl RowDescription {
    pub fn requires_rewrite(&self) -> bool {
        self.fields.iter().any(|f| f.requires_rewrite())
    }

    pub fn map_types(&mut self, projection_types: &[Option<Type>]) {
        self.fields
            .iter_mut()
            .zip(projection_types.iter())
            .for_each(|(field, t)| {
                if let Some(t) = t {
                    field.rewrite_type_oid(t.clone());
                }
            });
    }
}

impl RowDescriptionField {
    pub fn rewrite_type_oid(&mut self, postgres_type: postgres_types::Type) {
        self.type_oid = postgres_type.oid() as i32;
        self.dirty = true;
    }

    pub fn requires_rewrite(&self) -> bool {
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
        let num_fields = cursor.get_i16() as usize;

        let fields = std::iter::repeat_with(|| RowDescriptionField::try_from(&mut cursor))
            .take(num_fields)
            .collect::<Result<_, _>>()?;

        Ok(RowDescription { fields })
    }
}

impl TryFrom<RowDescription> for BytesMut {
    type Error = Error;

    fn try_from(row_description: RowDescription) -> Result<BytesMut, Error> {
        let mut bytes = BytesMut::new();

        // Convert each field to bytes
        let fields = row_description
            .fields
            .into_iter()
            .map(BytesMut::try_from)
            .collect::<Result<Vec<_>, _>>()?;

        let field_count = fields.len();
        let field_size = fields.iter().map(|x| x.len()).sum::<usize>();

        let len = SIZE_I32 + SIZE_I16 + field_size;

        bytes.put_u8(BackendCode::RowDescription.into());
        bytes.put_i32(len as i32);
        bytes.put_i16(field_count as i16);

        for field in fields.into_iter() {
            bytes.put_slice(&field);
        }

        Ok(bytes)
    }
}

// impl TryFrom<&BytesMut> for RowDescriptionField {
impl TryFrom<&mut Cursor<&BytesMut>> for RowDescriptionField {
    type Error = Error;

    fn try_from(cursor: &mut Cursor<&BytesMut>) -> Result<RowDescriptionField, Self::Error> {
        let name = cursor.read_string()?;

        let table_oid = cursor.get_i32();
        let table_column = cursor.get_i16();
        let type_oid = cursor.get_i32();

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
        bytes.put_i32(field.type_oid);
        bytes.put_i16(field.type_size);
        bytes.put_i32(field.type_modifier);
        bytes.put_i16(field.format_code.into());

        Ok(bytes)
    }
}

#[cfg(test)]
mod tests {

    use crate::{config::LogConfig, log, postgresql::messages::row_description::RowDescription};
    use bytes::BytesMut;
    use tracing::info;

    fn to_message(s: &[u8]) -> BytesMut {
        BytesMut::from(s)
    }

    #[test]
    pub fn map_projection_types() {
        log::init(LogConfig::default());

        // let mut pd = RowDescription {
        //     types: vec![
        //         postgres_types::Type::TEXT,
        //         postgres_types::Type::INT4,
        //         postgres_types::Type::INT8,
        //     ],
        // };

        // let mapped_types = vec![
        //     Some(postgres_types::Type::TEXT),
        //     None,
        //     Some(postgres_types::Type::TEXT),
        // ];

        // pd.map_types(&mapped_types);

        // let expected = vec![
        //     postgres_types::Type::TEXT,
        //     postgres_types::Type::INT4,
        //     postgres_types::Type::TEXT,
        // ];

        // assert_eq!(pd.types, expected);
    }

    #[test]
    pub fn parse_row_description() {
        log::init(LogConfig::default());
        let bytes = to_message(
            b"T\0\0\0!\0\x01TimeZone\0\0\0\0\0\0\0\0\0\0\x19\xff\xff\xff\xff\xff\xff\0\0",
        );

        let expected = bytes.clone();

        let row_description = RowDescription::try_from(&bytes).unwrap();

        info!("{:?}", row_description);

        assert_eq!(row_description.fields.len(), 1);
        assert_eq!(row_description.fields[0].name, "TimeZone");

        let bytes = BytesMut::try_from(row_description).unwrap();
        assert_eq!(bytes, expected);
    }

    #[test]
    pub fn parse_row_description_with_many_fields() {
        log::init(LogConfig::default());
        let bytes = to_message(
             b"T\0\0\0J\0\x03id\0\0\0h,\0\x01\0\0\0\x14\0\x08\xff\xff\xff\xff\0\0name\0\0\0h,\0\x02\0\0\0\x19\xff\xff\xff\xff\xff\xff\0\0email\0\0\0h,\0\x03\0\0\x0e\xda\xff\xff\xff\xff\xff\xff\0\0"
        );

        let expected = bytes.clone();

        let row_description = RowDescription::try_from(&bytes).unwrap();

        assert_eq!(row_description.fields.len(), 3);
        assert_eq!(row_description.fields[0].name, "id");
        assert_eq!(row_description.fields[1].name, "name");
        assert_eq!(row_description.fields[2].name, "email");

        let bytes = BytesMut::try_from(row_description).unwrap();
        assert_eq!(bytes, expected);
    }
}
