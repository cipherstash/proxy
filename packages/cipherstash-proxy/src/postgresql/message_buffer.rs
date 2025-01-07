use super::messages::data_row::DataRow;

pub struct MessageBuffer {
    // buffer: RwLock<Vec<DataRow>>,
    buffer: Vec<DataRow>,
}

impl MessageBuffer {
    /// Default number of rows to keep in the buffer.
    /// Larger rows will require more memory.
    const DEFAULT_RESPONSE_BUFFER_SIZE: usize = 4096;

    pub fn new() -> Self {
        Self {
            buffer: Vec::with_capacity(Self::DEFAULT_RESPONSE_BUFFER_SIZE),
        }
    }

    pub async fn push(&mut self, row: DataRow) {
        self.buffer.push(row);
    }

    pub async fn drain(&mut self) -> Vec<DataRow> {
        self.buffer.drain(..).collect()
    }

    pub async fn clear(&mut self) {
        self.buffer.clear();
    }

    pub async fn len(&self) -> usize {
        self.buffer.len()
    }

    // TODO: Consider treating capacity as the number of columns or memory used
    pub async fn at_capacity(&self) -> bool {
        self.buffer.len() >= Self::DEFAULT_RESPONSE_BUFFER_SIZE
    }
}
