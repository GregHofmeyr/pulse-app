//! No desktop notifications on this platform.

use super::{Failure, Notice, OnClick};

pub struct Backend;

impl Backend {
    pub fn new(_app_id: String, _on_click: OnClick) -> Self {
        Backend
    }

    pub fn show(&mut self, _n: &Notice) -> Result<(), Failure> {
        Ok(())
    }
}
