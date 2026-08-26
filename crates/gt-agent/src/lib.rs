mod capability;
mod collaboration;
mod models;
mod policy;
mod repository;

pub use capability::*;
pub use collaboration::*;
pub use models::*;
pub use policy::*;
pub use repository::*;

pub fn module_name() -> &'static str {
    "gt-agent"
}
