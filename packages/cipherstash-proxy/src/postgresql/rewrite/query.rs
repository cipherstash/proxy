//! CipherStash simple Query rewriting.
use bytes::Bytes;
use pg_proto::FrontendMessage;

#[derive(Debug, Clone)]
pub struct Query {
    pub statement: String,
    // Used to mark that a Query message requires rewrite
    dirty: bool,
}

impl Query {
    pub fn new(statement: String) -> Self {
        Self {
            statement,
            dirty: false,
        }
    }

    pub fn requires_rewrite(&self) -> bool {
        self.dirty
    }

    pub fn rewrite(&mut self, statement: String) {
        self.statement = statement;
        self.dirty = true;
    }
}

impl From<Bytes> for Query {
    fn from(query: Bytes) -> Self {
        Self {
            statement: String::from_utf8_lossy(&query).into_owned(),
            dirty: false,
        }
    }
}

impl From<Query> for FrontendMessage {
    fn from(query: Query) -> Self {
        Self::Query(Bytes::from(query.statement))
    }
}
