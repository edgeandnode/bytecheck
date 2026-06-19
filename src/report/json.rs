//! JSON serialization of a `Report` (all sections derive `Serialize`).

use super::Report;
use crate::Error;

pub fn render(report: &Report) -> Result<String, Error> {
    serde_json::to_string_pretty(report).map_err(|e| Error::Operational(format!("json: {e}")))
}
