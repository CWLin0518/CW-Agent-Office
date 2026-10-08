mod capability;
mod collaboration;
mod communication;
mod models;
mod output_contract;
mod policy;
mod repository;

pub use capability::*;
pub use collaboration::*;
pub use communication::*;
pub use models::*;
pub use output_contract::*;
pub use policy::*;
pub use repository::*;

pub fn module_name() -> &'static str {
    "gt-agent"
}
