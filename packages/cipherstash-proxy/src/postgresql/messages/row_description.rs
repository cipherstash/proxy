use bytes::Bytes;
#[cfg(test)]
use bytes::BytesMut;
use pg_proto::codec::{
    BackendMessage, FieldDescription as PgFieldDescription, RowDescription as PgRowDescription,
};
use postgres_types::Type;

use crate::postgresql::format_code::FormatCode;
#[cfg(test)]
use crate::{
    error::{Error, ProtocolError},
    postgresql::test_codec::{decode_backend_frame, encode_backend_message},
};

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

#[cfg(test)]
impl TryFrom<&BytesMut> for RowDescription {
    type Error = Error;

    fn try_from(bytes: &BytesMut) -> Result<RowDescription, Error> {
        let BackendMessage::RowDescription(description) = decode_backend_frame(bytes)? else {
            return Err(ProtocolError::UnexpectedMessageCode {
                expected: 'T',
                received: bytes.first().copied().unwrap_or_default() as char,
            }
            .into());
        };

        let fields = description
            .fields
            .into_iter()
            .map(|field| RowDescriptionField {
                name: String::from_utf8_lossy(&field.name).into_owned(),
                table_oid: field.table_oid as i32,
                table_column: field.column,
                type_oid: field.type_oid as i32,
                type_size: field.type_size,
                type_modifier: field.type_modifier,
                format_code: field.format.into(),
                dirty: false,
            })
            .collect();

        Ok(RowDescription { fields })
    }
}

impl From<PgRowDescription> for RowDescription {
    fn from(description: PgRowDescription) -> Self {
        Self {
            fields: description
                .fields
                .into_iter()
                .map(|field| RowDescriptionField {
                    name: String::from_utf8_lossy(&field.name).into_owned(),
                    table_oid: field.table_oid as i32,
                    table_column: field.column,
                    type_oid: field.type_oid as i32,
                    type_size: field.type_size,
                    type_modifier: field.type_modifier,
                    format_code: field.format.into(),
                    dirty: false,
                })
                .collect(),
        }
    }
}

#[cfg(test)]
impl TryFrom<RowDescription> for BytesMut {
    type Error = Error;

    fn try_from(row_description: RowDescription) -> Result<BytesMut, Error> {
        let fields = row_description
            .fields
            .into_iter()
            .map(|field| PgFieldDescription {
                name: Bytes::from(field.name),
                table_oid: field.table_oid as u32,
                column: field.table_column,
                type_oid: field.type_oid as u32,
                type_size: field.type_size,
                type_modifier: field.type_modifier,
                format: field.format_code.into(),
            })
            .collect();

        encode_backend_message(&BackendMessage::RowDescription(PgRowDescription { fields }))
    }
}

impl From<RowDescription> for BackendMessage {
    fn from(row_description: RowDescription) -> Self {
        Self::RowDescription(PgRowDescription {
            fields: row_description
                .fields
                .into_iter()
                .map(|field| PgFieldDescription {
                    name: Bytes::from(field.name),
                    table_oid: field.table_oid as u32,
                    column: field.table_column,
                    type_oid: field.type_oid as u32,
                    type_size: field.type_size,
                    type_modifier: field.type_modifier,
                    format: field.format_code.into(),
                })
                .collect(),
        })
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
