mod models;
mod policy;
mod repository;

pub use models::*;
pub use policy::*;
pub use repository::*;

pub fn module_name() -> &'static str {
    "gt-agent"
}
