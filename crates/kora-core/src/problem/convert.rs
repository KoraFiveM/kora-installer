//! Conversions from Kora's internal error enums into a [`Problem`]. Callers that `match` on these
//! enums keep doing so; the conversion happens where a person could see the error (`?` in a function
//! that returns [`super::Res`]).
//!
//! The enums below already have `Display` text written as a sentence for a person, so the sentence
//! is the message. [`GitError`](crate::git::GitError) is different: it holds command lines and raw
//! stderr, so its text goes into `details` and the message is a plain sentence.

use super::{codes, Problem};

macro_rules! sentence {
    ($($t:ty),+ $(,)?) => {$(
        impl From<$t> for Problem {
            fn from(e: $t) -> Problem {
                Problem::generic(e.to_string())
            }
        }
    )+};
}

sentence!(
    crate::notify::NotifyError,
    crate::service::ServiceError,
    crate::update::UpdateError,
    crate::catalog::CatalogError,
    crate::artifacts::ArtifactError,
    crate::app_install::AppError,
    crate::db::DbError,
    crate::hub::remote::RemoteError,
);

impl From<crate::git::GitError> for Problem {
    fn from(e: crate::git::GitError) -> Problem {
        use crate::git::GitError as G;
        match &e {
            G::Spawn(..) => Problem::new(codes::GIT_FAILED, "Kora could not run git or the GitHub CLI.").with_details(e.to_string()),
            G::Command { .. } => Problem::new(codes::GIT_FAILED, "A git or GitHub command did not work.").with_details(e.to_string()),
            G::Invalid(..) | G::Parse(..) => Problem::new(codes::GIT_FAILED, "Git or GitHub answered with something Kora did not understand.").with_details(e.to_string()),
        }
    }
}
