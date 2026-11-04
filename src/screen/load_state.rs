//! Per-screen load state.
//!
//! Not Found is a state, not a route: deleting the open album externally must
//! leave the user on the same navigation entry and render an in-place message.

use crate::library::LibraryError;

#[derive(Debug, Clone)]
pub enum LoadState<T> {
    Loading,
    Loaded(T),
    NotFound,
    Error(String),
}

impl<T> LoadState<T> {
    pub fn is_loading(&self) -> bool {
        matches!(self, Self::Loading)
    }

    pub fn loaded(&self) -> Option<&T> {
        match self {
            Self::Loaded(value) => Some(value),
            _ => None,
        }
    }

    pub fn loaded_mut(&mut self) -> Option<&mut T> {
        match self {
            Self::Loaded(value) => Some(value),
            _ => None,
        }
    }

    pub fn error(&self) -> Option<&str> {
        match self {
            Self::Error(error) => Some(error),
            _ => None,
        }
    }
}

/// Library errors split into the one state the UI renders differently.
#[derive(Debug, Clone)]
pub enum LoadError {
    NotFound,
    Other(String),
}

impl From<LibraryError> for LoadError {
    fn from(err: LibraryError) -> Self {
        match err {
            LibraryError::NotFound(message) => {
                let _ = message;
                Self::NotFound
            }
            other => Self::Other(other.to_string()),
        }
    }
}

pub fn load_state<T>(result: Result<T, LoadError>) -> LoadState<T> {
    match result {
        Ok(value) => LoadState::Loaded(value),
        Err(LoadError::NotFound) => LoadState::NotFound,
        Err(LoadError::Other(error)) => LoadState::Error(error),
    }
}

#[cfg(test)]
mod tests {
    use crate::library::LibraryError;

    use super::*;

    #[test]
    fn not_found_is_a_state_not_an_error() {
        let state: LoadState<()> = load_state(Err(LibraryError::NotFound("gone".into()).into()));
        assert!(matches!(state, LoadState::NotFound));
    }

    #[test]
    fn other_errors_keep_their_message() {
        let state: LoadState<()> = load_state(Err(LibraryError::Invalid("bad".into()).into()));
        match state {
            LoadState::Error(message) => assert_eq!(message, "bad"),
            other => panic!("expected Error, got {other:?}"),
        }
    }

    #[test]
    fn loaded_values_are_reachable() {
        let mut state = load_state::<u32>(Ok(7));
        assert_eq!(state.loaded(), Some(&7));
        *state.loaded_mut().expect("loaded") = 8;
        assert_eq!(state.loaded(), Some(&8));
    }
}
