#[derive(Debug, Clone, Hash, Eq, PartialEq)]
pub enum Name {
    Named(String),
    Unnamed,
}

impl Name {
    pub fn unnamed() -> Name {
        Name::Unnamed
    }

    pub fn is_unnamed(&self) -> bool {
        matches!(self, Name::Unnamed)
    }

    pub fn as_str(&self) -> &str {
        match self {
            Name::Named(s) => s,
            Name::Unnamed => "",
        }
    }
}

impl std::ops::Deref for Name {
    type Target = str;

    fn deref(&self) -> &str {
        self.as_str()
    }
}

impl From<String> for Name {
    fn from(s: String) -> Self {
        if s.is_empty() {
            Name::Unnamed
        } else {
            Name::Named(s)
        }
    }
}

impl From<&str> for Name {
    fn from(s: &str) -> Self {
        if s.is_empty() {
            Name::Unnamed
        } else {
            Name::Named(s.to_string())
        }
    }
}
