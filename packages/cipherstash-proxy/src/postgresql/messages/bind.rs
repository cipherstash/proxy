use super::{maybe_json, maybe_jsonb, Name, NULL};
use crate::error::{Error, MappingError, ProtocolError};
use crate::log::MAPPER;
use crate::postgresql::context::column::Column;
use crate::postgresql::context::statement::{
    params_are_positional, JsonSelectorPath, JsonSelectorStep, OutputParam, OutputParamSource,
};
use crate::postgresql::data::{
    bind_param_from_sql, bind_param_json_value, json_value_selector_plaintext,
};
use crate::postgresql::format_code::FormatCode;
use crate::postgresql::protocol::BytesMutReadString;
use crate::{EqlOutput, EqlQueryPayload};
use crate::{SIZE_I16, SIZE_I32};
use bytes::{Buf, BufMut, BytesMut};
use cipherstash_client::encryption::Plaintext;
use postgres_types::Type;
use std::fmt::{self, Display, Formatter};
use std::io::Cursor;
use std::{convert::TryFrom, ffi::CString};
use tracing::debug;

/// Bind (B) message.
/// See: <https://www.postgresql.org/docs/current/protocol-message-formats.html>
#[derive(Clone, Debug)]
pub struct Bind {
    pub code: char,
    pub portal: Name,
    pub prepared_statement: Name,
    pub num_param_format_codes: i16,
    pub param_format_codes: Vec<FormatCode>,
    pub num_param_values: i16,
    pub param_values: Vec<BindParam>,
    pub num_result_column_format_codes: i16,
    pub result_columns_format_codes: Vec<FormatCode>,
    /// Set when the param list was rebuilt because the rewrite reshaped the
    /// params. The message must then be re-sent even if no individual param was
    /// itself edited, because the count and framing changed.
    reshaped: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct BindParam {
    pub format_code: FormatCode,
    pub bytes: BytesMut,
    dirty: bool,
}

impl Bind {
    pub fn requires_rewrite(&self) -> bool {
        self.reshaped
            || self
                .param_values
                .iter()
                .any(|param| param.requires_rewrite())
    }

    /// Converts the bound params to the plaintexts of the **output** params —
    /// the values PostgreSQL will receive, which are not necessarily the values
    /// the client bound.
    ///
    /// Each output param pulls from the input param(s) its `source` names, so a
    /// fused JSON value selector reads both halves here and a dropped path
    /// operand is never decoded on its own (its bytes are only half a needle and
    /// would not decode as a standalone operand for the column).
    pub fn to_plaintext(
        &self,
        output_params: &[OutputParam],
        param_types: &[i32],
    ) -> Result<Vec<Option<Plaintext>>, Error> {
        output_params
            .iter()
            .map(|output| {
                let Some(col) = &output.column else {
                    // Native param: forwarded verbatim, nothing to encrypt.
                    return Ok(None);
                };

                let input = output.source.primary_input();
                let bound_param_type = get_param_type(input, param_types, col);

                debug!(
                    target: MAPPER,
                    col = ?col, bound_param_type = ?bound_param_type, ?input
                );

                match &output.source {
                    OutputParamSource::Input(idx) => {
                        let Some(param) = self.param_values.get(*idx) else {
                            return Ok(None);
                        };

                        // Convert param bytes into a Plaintext wrapping a Value
                        // If the param type is different, will convert the bound type to the correct Plaintext variant identified by the cast_type
                        bind_param_from_sql(
                            param,
                            &bound_param_type,
                            col.eql_term(),
                            col.cast_type(),
                        )
                        .map_err(|_| {
                            MappingError::InvalidParameter(Box::new(col.to_owned())).into()
                        })
                    }
                    OutputParamSource::JsonValueSelector { path, value } => {
                        self.json_value_selector_plaintext(path, *value, &bound_param_type)
                    }
                }
            })
            .collect()
    }

    /// Composes `{"path", "value"}` — the input to `SteVecValueSelector` — from
    /// the operands of a JSON field equality.
    ///
    /// Each step of the path is either a literal from the SQL or another bind
    /// param, which is read straight off the wire: it is the selector *text*, so
    /// it needs none of the per-column decoding the value half goes through.
    ///
    /// A NULL step (or a NULL value) yields no needle: `col -> NULL = x` is NULL
    /// in SQL, so there is nothing to match. The caller must then bind NULL —
    /// forwarding the operand the client sent would put it on the wire in
    /// plaintext.
    fn json_value_selector_plaintext(
        &self,
        path: &JsonSelectorPath,
        value: usize,
        postgres_type: &Type,
    ) -> Result<Option<Plaintext>, Error> {
        let mut steps = Vec::with_capacity(path.steps.len());

        for step in &path.steps {
            match step {
                JsonSelectorStep::Literal(selector) => steps.push(selector.to_owned()),
                JsonSelectorStep::Param(step_idx) => match self.param_values.get(*step_idx) {
                    Some(param) if !param.is_null() => steps.push(param.to_string()),
                    _ => return Ok(None),
                },
            }
        }

        let Some(param) = self.param_values.get(value) else {
            return Ok(None);
        };

        let Some(value) = bind_param_json_value(param, postgres_type)? else {
            return Ok(None);
        };

        debug!(
            target: MAPPER,
            msg = "Fused JSON value selector",
            path = ?steps,
            ?value
        );

        let steps: Vec<&str> = steps.iter().map(String::as_str).collect();

        Ok(Some(json_value_selector_plaintext(&steps, value)?))
    }

    /// Replaces the bound params with the output params of the rewritten
    /// statement.
    ///
    /// When the plan is positional (the overwhelmingly common case) the params
    /// are patched in place, leaving the client's framing — including its format
    /// code encoding — exactly as sent. When the rewrite reshaped the params,
    /// the list is rebuilt: each output param inherits the wire bytes and format
    /// code of the input it was built around, and an explicit format code is
    /// emitted per param since the counts no longer line up.
    pub fn rewrite(
        &mut self,
        output_params: &[OutputParam],
        encrypted: Vec<Option<EqlOutput>>,
    ) -> Result<(), Error> {
        if output_params.len() == self.param_values.len() && params_are_positional(output_params) {
            for ((param, output), ct) in self
                .param_values
                .iter_mut()
                .zip(output_params.iter())
                .zip(encrypted.iter())
            {
                Self::apply_output(param, output, ct.as_ref())?;
            }
            return Ok(());
        }

        let mut param_values = Vec::with_capacity(output_params.len());
        for (output, ct) in output_params.iter().zip(encrypted.iter()) {
            let input = output.source.primary_input();
            let mut param = self.param_values.get(input).cloned().ok_or(
                ProtocolError::MissingBoundParameter {
                    param: input + 1,
                    received: self.param_values.len(),
                },
            )?;

            Self::apply_output(&mut param, output, ct.as_ref())?;

            param_values.push(param);
        }

        self.param_format_codes = param_values.iter().map(|param| param.format_code).collect();
        self.num_param_format_codes = self.param_format_codes.len() as i16;
        self.num_param_values = param_values.len() as i16;
        self.param_values = param_values;
        self.reshaped = true;

        Ok(())
    }

    /// Writes what PostgreSQL receives for one output param.
    ///
    /// An output param the plan says must be ENCRYPTED, but for which no
    /// ciphertext was produced, is bound NULL. Its bytes are the client's
    /// plaintext operand, so leaving them in place would send it to the
    /// database: that is the shape a fusion takes when it cannot build a needle
    /// (`col -> NULL = $1`), where the operand went unencrypted precisely
    /// because there is nothing to match. NULL is also what the SQL means — a
    /// comparison against NULL is NULL — so the predicate correctly returns no
    /// rows.
    fn apply_output(
        param: &mut BindParam,
        output: &OutputParam,
        ct: Option<&EqlOutput>,
    ) -> Result<(), Error> {
        if output.column.is_some() && ct.is_none() {
            param.rewrite_null();
            return Ok(());
        }

        Self::apply_encrypted(param, ct)
    }

    fn apply_encrypted(param: &mut BindParam, ct: Option<&EqlOutput>) -> Result<(), Error> {
        match ct {
            // A JSON selector (`->`/`->>`/`jsonb_path_query`) is a bare
            // tokenized-selector hash bound directly as `text`, NOT jsonb.
            // Use the raw token: JSON-serializing it re-quotes the bare
            // string (`"<hash>"`), which never matches the stored per-entry
            // `s`. It must also skip the jsonb version header a binary
            // rewrite would prepend — the binary wire form of `text` is
            // just its raw bytes, and a leading `0x01` corrupts the
            // selector so `->` matches nothing.
            Some(EqlOutput::Query(EqlQueryPayload::Selector(s))) => {
                param.rewrite_text(s.clone().into_bytes());
            }
            // convert json to bytes
            Some(ct) => {
                let bytes = serde_json::to_value(ct)?.to_string().into_bytes();
                param.rewrite(&bytes);
            }
            None => {}
        }

        Ok(())
    }
}

///
/// Param type is either provided with Parse message or the column type
/// Column type is the cast of the encrypted column
///
fn get_param_type(idx: usize, param_types: &[i32], col: &Column) -> Type {
    param_types
        .get(idx)
        .and_then(|oid| Type::from_oid(*oid as u32))
        .unwrap_or_else(|| col.postgres_type.clone())
}

impl BindParam {
    pub fn new(format_code: FormatCode, bytes: BytesMut) -> Self {
        Self {
            format_code,
            bytes,
            dirty: false,
        }
    }

    pub fn null() -> Self {
        Self {
            format_code: FormatCode::Text,
            bytes: BytesMut::new(),
            dirty: false,
        }
    }

    ///
    /// Returns the actual length of the param bytes
    /// The actual byte length needs to be used when calculating the Bind message length
    /// If NULL returns 0 as the actual byte length
    /// Not to be confused with the *param* len as encoded in the Bind message
    ///
    pub fn byte_len(&self) -> usize {
        self.bytes.len()
    }

    ///
    /// Returns the length of the param for representation in the Bind message
    /// If NULL returns -1 as required by the PostgreSQL protocol
    ///
    pub fn len(&self) -> i32 {
        if self.is_null() {
            return NULL;
        }
        self.bytes.len() as i32
    }

    pub fn rewrite(&mut self, bytes: &[u8]) {
        self.bytes.clear();

        if self.is_binary() {
            self.bytes.put_u8(1);
        }

        self.bytes.extend_from_slice(bytes);
        self.dirty = true;
    }

    /// Rewrite this param as a bare `text` value, without the jsonb version
    /// header [`rewrite`] prepends for binary jsonb payloads.
    ///
    /// Used for the tokenized selector of a JSON field access (`->`/`->>`/
    /// `jsonb_path_query`), which is bound as `text`, not jsonb. The binary
    /// wire form of `text` is simply its raw UTF-8 bytes, so no header is
    /// added in either format — a stray `0x01` would corrupt the selector and
    /// stop `->` from matching any stored entry.
    pub fn rewrite_text(&mut self, bytes: Vec<u8>) {
        self.bytes.clear();
        self.bytes.extend_from_slice(&bytes);
        self.dirty = true;
    }

    /// Rewrite this param as NULL, discarding whatever the client bound.
    ///
    /// Used for an encrypted operand that produced no ciphertext: its bytes are
    /// plaintext, so they must not reach the database. An already-NULL param is
    /// left alone rather than marked dirty — there is nothing to replace, and
    /// dirtying it would re-send a Bind message that has not changed.
    pub fn rewrite_null(&mut self) {
        if self.is_null() {
            return;
        }

        self.bytes.clear();
        self.dirty = true;
    }

    pub fn requires_rewrite(&self) -> bool {
        self.dirty
    }

    pub fn maybe_plaintext(&self) -> bool {
        self.is_text() && maybe_json(&self.bytes) || self.is_binary() && maybe_jsonb(&self.bytes)
    }

    ///
    /// If the text format is binary, returns a reference to the bytes without the jsonb header byte
    ///
    pub fn json_bytes(&self) -> &[u8] {
        if self.is_binary() {
            &self.bytes[1..]
        } else {
            &self.bytes[0..]
        }
    }

    pub fn is_null(&self) -> bool {
        self.bytes.is_empty()
    }

    pub fn is_text(&self) -> bool {
        self.format_code == FormatCode::Text
    }

    pub fn is_binary(&self) -> bool {
        self.format_code == FormatCode::Binary
    }
}

impl Display for BindParam {
    fn fmt(&self, f: &mut Formatter) -> fmt::Result {
        let s = String::from_utf8_lossy(&self.bytes).to_string();
        write!(f, "{s}")
    }
}

impl TryFrom<&BytesMut> for Bind {
    type Error = Error;

    fn try_from(buf: &BytesMut) -> Result<Bind, Self::Error> {
        let mut cursor = Cursor::new(buf);
        let code = cursor.get_u8() as char;
        let _len = cursor.get_i32();

        let portal = cursor.read_string()?;
        let portal = Name::from(portal);

        let prepared_statement = cursor.read_string()?;
        let prepared_statement = Name::from(prepared_statement);

        let num_param_format_codes = cursor.get_i16();
        let mut param_format_codes = Vec::new();

        for _ in 0..num_param_format_codes {
            param_format_codes.push(cursor.get_i16().into());
        }

        let num_param_values = cursor.get_i16();
        let mut param_values = Vec::new();

        for idx in 0..num_param_values as usize {
            let param_len = cursor.get_i32();

            let format_code = match num_param_format_codes {
                0 => FormatCode::Text,
                1 => param_format_codes[0],
                _ => param_format_codes[idx],
            };

            // NULL parameters have a length of -1 and no bytes
            match param_len {
                NULL => {
                    param_values.push(BindParam::null());
                }
                _ => {
                    let mut bytes = BytesMut::with_capacity(param_len as usize);
                    bytes.resize(param_len as usize, b'0');
                    cursor.copy_to_slice(&mut bytes);
                    param_values.push(BindParam::new(format_code, bytes));
                }
            }
        }

        let num_result_column_format_codes = cursor.get_i16();
        let mut result_columns_format_codes = Vec::new();

        for _ in 0..num_result_column_format_codes {
            result_columns_format_codes.push(cursor.get_i16().into());
        }

        Ok(Bind {
            code,
            portal,
            prepared_statement,
            num_param_format_codes,
            param_format_codes,
            num_param_values,
            param_values,
            num_result_column_format_codes,
            result_columns_format_codes,
            reshaped: false,
        })
    }
}

impl TryFrom<Bind> for BytesMut {
    type Error = Error;

    fn try_from(bind: Bind) -> Result<BytesMut, Self::Error> {
        let mut bytes = BytesMut::new();

        let portal_binding = CString::new(&*bind.portal)?;
        let portal = portal_binding.as_bytes_with_nul();

        let prepared_statement_binding = CString::new(&*bind.prepared_statement)?;
        let prepared_statement = prepared_statement_binding.as_bytes_with_nul();

        if bind.num_param_format_codes != bind.param_format_codes.len() as i16 {
            let err = ProtocolError::ParameterFormatCodesMismatch {
                expected: bind.num_param_format_codes as usize,
                received: bind.param_format_codes.len(),
            };
            return Err(err.into());
        }

        if bind.num_result_column_format_codes != bind.result_columns_format_codes.len() as i16 {
            let err = ProtocolError::ParameterResultFormatCodesMismatch {
                expected: bind.num_result_column_format_codes as usize,
                received: bind.result_columns_format_codes.len(),
            };
            return Err(err.into());
        }

        // sum of param byte_lens (the *actual* byte lengths of the parameters)
        let param_byte_len = &bind
            .param_values
            .iter()
            .fold(0, |acc, param| acc + SIZE_I32 + param.byte_len());

        let len = SIZE_I32 // self/len of len
            + portal.len()
            + prepared_statement.len()
            + SIZE_I16 // num_param_format_codes
            + SIZE_I16 * bind.num_param_format_codes as usize // num_param_format_codes
            + SIZE_I16  // num_param_values
            + param_byte_len // parameter bytes
            + SIZE_I16 // num_result_column_format_codes
            + SIZE_I16 * bind.num_result_column_format_codes as usize;

        bytes.put_u8(bind.code as u8);
        bytes.put_i32(len as i32);
        bytes.put_slice(portal);
        bytes.put_slice(prepared_statement);
        bytes.put_i16(bind.num_param_format_codes);
        for param_format_code in bind.param_format_codes {
            bytes.put_i16(param_format_code.into());
        }

        let num_param_values = bind.param_values.len() as i16;
        bytes.put_i16(num_param_values);

        for p in bind.param_values {
            // len is not the same as byte_len
            // A NULL param len is -1
            bytes.put_i32(p.len());
            bytes.put_slice(&p.bytes);
        }

        bytes.put_i16(bind.num_result_column_format_codes);
        for result_column_format_code in bind.result_columns_format_codes {
            bytes.put_i16(result_column_format_code.into());
        }

        Ok(bytes)
    }
}

#[cfg(test)]
mod tests {
    use super::{BindParam, JsonSelectorPath, JsonSelectorStep, OutputParam, OutputParamSource};
    use crate::{
        config::LogConfig,
        log,
        postgresql::{
            context::column::Column, format_code::FormatCode, messages::bind::Bind, messages::Name,
        },
        Identifier,
    };
    use bytes::BytesMut;
    use cipherstash_client::schema::{ColumnConfig, ColumnMode, ColumnType};
    use eql_mapper::EqlTermVariant;

    fn to_message(s: &[u8]) -> BytesMut {
        BytesMut::from(s)
    }

    #[test]
    pub fn parse_bind() {
        log::init(LogConfig::default());
        let bytes =
            to_message(b"B\0\0\0\x18\0\0\0\x01\0\x01\0\x01\0\0\0\x04.\xbe\x8a\xd4\0\x01\0\x01");

        let expected = bytes.clone();

        let bind = Bind::try_from(&bytes).unwrap();

        assert_eq!(bind.param_values.len(), 1);
        assert_eq!(bind.result_columns_format_codes[0], FormatCode::Binary);

        let bytes = BytesMut::try_from(bind).unwrap();
        assert_eq!(bytes, expected);
    }

    #[test]
    pub fn parse_bind_with_null_param() {
        log::init(LogConfig::default());

        // Bind message from statement INSERT INTO encrypted (id, plaintext, plaintext_date, encrypted_text) VALUES ($1, $2, $3, $4)
        let bytes =
            to_message(b"B\0\0\0N\0s0\0\0\x04\0\x01\0\x01\0\x01\0\x01\0\x04\0\0\0\x084\xd8\x1d@\x83U\x0em\0\0\0\tplaintext\xff\xff\xff\xff\0\0\0\x15hello@cipherstash.com\0\x01\0\x01");

        let expected = bytes.clone();

        let bind = Bind::try_from(&bytes).unwrap();

        assert_eq!(bind.param_values.len(), 4);

        let bytes = BytesMut::try_from(bind).unwrap();
        assert_eq!(bytes, expected);
    }

    #[test]
    fn bind_should_rewrite() {
        log::init(LogConfig::default());

        let bytes = "hello".into();
        let mut param = BindParam::new(FormatCode::Text, bytes);

        param.rewrite("world".as_bytes());

        assert!(param.requires_rewrite());
    }

    fn text_param(value: &str) -> BindParam {
        BindParam::new(FormatCode::Text, BytesMut::from(value.as_bytes()))
    }

    fn encrypted_column() -> Column {
        Column {
            identifier: Identifier::new("encrypted", "encrypted_jsonb"),
            config: ColumnConfig {
                name: "encrypted_jsonb".to_owned(),
                in_place: false,
                cast_type: ColumnType::Json,
                indexes: vec![],
                mode: ColumnMode::PlaintextDuplicate,
            },
            postgres_type: postgres_types::Type::JSONB,
            eql_term: EqlTermVariant::JsonValueSelector,
        }
    }

    fn bind_with(param_values: Vec<BindParam>) -> Bind {
        Bind {
            code: 'B',
            portal: Name::unnamed(),
            prepared_statement: Name::unnamed(),
            num_param_format_codes: param_values.len() as i16,
            param_format_codes: param_values.iter().map(|p| p.format_code).collect(),
            num_param_values: param_values.len() as i16,
            param_values,
            num_result_column_format_codes: 0,
            result_columns_format_codes: vec![],
            reshaped: false,
        }
    }

    /// `col -> $1 = $2` with `$1` bound NULL builds no needle, so `$2` is never
    /// encrypted. Its bytes are the client's plaintext comparand: binding them
    /// would send the value to the database in the clear. NULL is bound instead,
    /// which is also what the SQL means.
    #[test]
    fn a_fusion_with_no_needle_binds_null_rather_than_the_clients_value() {
        log::init(LogConfig::default());

        let mut bind = bind_with(vec![BindParam::null(), text_param("\"world\"")]);

        let output_params = vec![OutputParam {
            column: Some(encrypted_column()),
            source: OutputParamSource::JsonValueSelector {
                path: JsonSelectorPath {
                    steps: vec![JsonSelectorStep::Param(0)],
                },
                value: 1,
            },
            query_operand: true,
        }];

        // What `to_plaintext` yields for this Bind: no needle, so no ciphertext.
        assert_eq!(
            vec![None],
            bind.to_plaintext(&output_params, &[]).unwrap(),
            "a NULL selector must not produce a needle"
        );

        bind.rewrite(&output_params, vec![None]).unwrap();

        assert_eq!(1, bind.param_values.len());
        assert!(
            bind.param_values[0].is_null(),
            "the value operand must be bound NULL, not forwarded: {:?}",
            bind.param_values[0].to_string()
        );
        assert!(bind.requires_rewrite());
    }

    /// The same holds when it is the VALUE that is NULL: nothing to encrypt, and
    /// nothing of the client's to forward.
    #[test]
    fn a_fusion_with_a_null_value_binds_null() {
        log::init(LogConfig::default());

        let mut bind = bind_with(vec![text_param("nested"), BindParam::null()]);

        let output_params = vec![OutputParam {
            column: Some(encrypted_column()),
            source: OutputParamSource::JsonValueSelector {
                path: JsonSelectorPath {
                    steps: vec![JsonSelectorStep::Param(0)],
                },
                value: 1,
            },
            query_operand: true,
        }];

        assert_eq!(vec![None], bind.to_plaintext(&output_params, &[]).unwrap());

        bind.rewrite(&output_params, vec![None]).unwrap();

        assert_eq!(1, bind.param_values.len());
        assert!(bind.param_values[0].is_null());
    }

    /// A NATIVE param has no column, so it is forwarded exactly as bound — the
    /// NULL rule is about operands that were supposed to be encrypted.
    #[test]
    fn a_native_param_is_still_forwarded_unchanged() {
        log::init(LogConfig::default());

        let mut bind = bind_with(vec![text_param("42"), text_param("plaintext")]);

        let output_params = vec![
            OutputParam {
                column: None,
                source: OutputParamSource::Input(0),
                query_operand: false,
            },
            OutputParam {
                column: None,
                source: OutputParamSource::Input(1),
                query_operand: false,
            },
        ];

        bind.rewrite(&output_params, vec![None, None]).unwrap();

        assert_eq!("42", bind.param_values[0].to_string());
        assert_eq!("plaintext", bind.param_values[1].to_string());
        assert!(!bind.requires_rewrite());
    }
}
