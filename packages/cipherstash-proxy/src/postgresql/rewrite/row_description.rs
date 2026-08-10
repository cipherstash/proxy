//! CipherStash RowDescription rewriting.
use bytes::Bytes;
use pg_proto::{
    BackendMessage, FieldDescription as PgFieldDescription, RowDescription as PgRowDescription,
};
use postgres_types::Type;

use crate::postgresql::format_code::FormatCode;

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

    use crate::{config::LogConfig, log, postgresql::rewrite::row_description::RowDescription};
    use bytes::Bytes;
    use pg_proto::{BackendMessage, FieldDescription, RowDescription as PgRowDescription};
    use tracing::info;

    fn field(
        name: &'static [u8],
        table_oid: u32,
        column: i16,
        type_oid: u32,
        type_size: i16,
    ) -> FieldDescription {
        FieldDescription {
            name: Bytes::from_static(name),
            table_oid,
            column,
            type_oid,
            type_size,
            type_modifier: -1,
            format: 0,
        }
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
        let expected = PgRowDescription {
            fields: vec![field(b"TimeZone", 0, 0, 25, -1)],
        };
        let row_description = RowDescription::from(expected.clone());

        info!("{:?}", row_description);

        assert_eq!(row_description.fields.len(), 1);
        assert_eq!(row_description.fields[0].name, "TimeZone");

        let BackendMessage::RowDescription(actual) = row_description.into() else {
            panic!("expected RowDescription")
        };
        assert_eq!(actual, expected);
    }

    #[test]
    pub fn parse_row_description_with_many_fields() {
        log::init(LogConfig::default());
        let expected = PgRowDescription {
            fields: vec![
                field(b"id", 26_668, 1, 20, 8),
                field(b"name", 26_668, 2, 25, -1),
                field(b"email", 26_668, 3, 3802, -1),
            ],
        };
        let row_description = RowDescription::from(expected.clone());

        assert_eq!(row_description.fields.len(), 3);
        assert_eq!(row_description.fields[0].name, "id");
        assert_eq!(row_description.fields[1].name, "name");
        assert_eq!(row_description.fields[2].name, "email");

        let BackendMessage::RowDescription(actual) = row_description.into() else {
            panic!("expected RowDescription")
        };
        assert_eq!(actual, expected);
    }
}
