//! Types shared by the Pulse server and client. TypeScript bindings are generated from these.

pub mod gateway;
pub mod ids;
pub mod rest;

use std::path::Path;

use ts_rs::{Config, ExportError, TS};

/// Write TypeScript bindings for every wire type into `dir`.
pub fn export_all(dir: &Path) -> Result<(), ExportError> {
    let cfg = Config::new().with_out_dir(dir).with_large_int("number");
    gateway::ClientFrame::export_all(&cfg)?;
    gateway::ServerFrame::export_all(&cfg)?;
    rest::RegisterRequest::export_all(&cfg)?;
    rest::LoginRequest::export_all(&cfg)?;
    rest::SessionResponse::export_all(&cfg)?;
    rest::InviteResponse::export_all(&cfg)?;
    rest::CreateServerRequest::export_all(&cfg)?;
    rest::CreateChannelRequest::export_all(&cfg)?;
    rest::CreateDmRequest::export_all(&cfg)?;
    rest::SendMessageRequest::export_all(&cfg)?;
    rest::EditMessageRequest::export_all(&cfg)?;
    rest::VoiceTokenResponse::export_all(&cfg)?;
    rest::ApiError::export_all(&cfg)?;
    rest::Person::export_all(&cfg)?;
    rest::ReadState::export_all(&cfg)?;
    rest::Mute::export_all(&cfg)?;
    rest::MuteTarget::export_all(&cfg)?;
    rest::AddMembersRequest::export_all(&cfg)?;
    rest::RenameChannelRequest::export_all(&cfg)?;
    rest::MarkReadRequest::export_all(&cfg)?;
    rest::SetMuteRequest::export_all(&cfg)?;
    Ok(())
}
