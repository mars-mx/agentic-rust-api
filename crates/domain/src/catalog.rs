use uuid::Uuid;

const TITLE_MAX_CHARS: usize = 200;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Item {
    pub id: Uuid,
    pub organization_id: Option<String>,
    pub title: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Title(String);

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum TitleError {
    #[error("title must not be empty")]
    Empty,
    #[error("title must be at most {TITLE_MAX_CHARS} characters")]
    TooLong,
}

impl Title {
    pub fn parse(raw: &str) -> Result<Self, TitleError> {
        let title = raw.trim();
        if title.is_empty() {
            Err(TitleError::Empty)
        } else if title.chars().count() > TITLE_MAX_CHARS {
            Err(TitleError::TooLong)
        } else {
            Ok(Self(title.to_owned()))
        }
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[cfg(test)]
mod tests {
    use super::{Title, TitleError};

    #[test]
    fn a_title_is_trimmed() {
        assert_eq!(Title::parse("  Lamp ").unwrap().as_str(), "Lamp");
    }

    #[test]
    fn an_empty_or_blank_title_is_rejected() {
        for raw in ["", "   "] {
            assert_eq!(Title::parse(raw), Err(TitleError::Empty), "{raw:?}");
        }
    }

    #[test]
    fn a_title_longer_than_the_limit_is_rejected() {
        assert_eq!(
            Title::parse(&"ä".repeat(200))
                .unwrap()
                .as_str()
                .chars()
                .count(),
            200
        );
        assert_eq!(Title::parse(&"ä".repeat(201)), Err(TitleError::TooLong));
    }
}
