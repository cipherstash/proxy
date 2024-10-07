use bytes::BytesMut;

#[derive(Debug, Clone)]
pub(crate) struct Parse {
    code: char,
    #[allow(dead_code)]
    len: i32,
    pub name: String,
    pub query: String,
    pub num_params: i16,
    pub param_types: Vec<i32>,
}

// impl TryFrom<&BytesMut> for Parse {
//     type Error = anyhow::Error;

//     fn try_from(bytes: &BytesMut) -> Result<Parse, Self::Error> {
//         let mut cursor = Cursor::new(bytes);
//         let code = cursor.get_u8();

//         if code != PARSE {
//             bail!("Invalid message code for Query {code}");
//         }

//         let _len = cursor.get_i32(); // read and progress cursor
//         let query = cursor.read_string()?;

//         // Ok(Parse { statement: query })
//     }
// }
