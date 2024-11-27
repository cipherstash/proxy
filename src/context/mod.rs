use std::collections::VecDeque;

#[derive(Debug, Clone)]
pub struct Context {
    statements: VecDeque<String>,
}

impl Context {
    pub fn new() -> Context {
        Context {
            statements: VecDeque::new(),
        }
    }

    pub fn add(&mut self, statement: String) {
        self.statements.push_back(statement);
    }

    pub fn pop(&mut self) -> Option<String> {
        self.statements.pop_front()
    }
}
