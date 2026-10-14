//! Application composition of the pure layout domain.
#[derive(Clone, Default)]
pub struct ApplicationLayoutState {
    pub application_layout: Option<crate::domain::application_layout::ApplicationLayout>,
    pub application_sash_drag: Option<crate::domain::application_layout::Divider>,
}
