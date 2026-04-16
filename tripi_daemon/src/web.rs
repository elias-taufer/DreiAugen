/*
SPDX-License-Identifier: GPL-3.0-or-later

    Copyright (C) 2026  Elias Taufer

    This program is free software: you can redistribute it and/or modify
    it under the terms of the GNU General Public License as published by
    the Free Software Foundation, either version 3 of the License, or
    (at your option) any later version.

    This program is distributed in the hope that it will be useful,
    but WITHOUT ANY WARRANTY; without even the implied warranty of
    MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
    GNU General Public License for more details.

    You should have received a copy of the GNU General Public License
    along with this program.  If not, see <https://www.gnu.org/licenses/>.
*/

use std::net::SocketAddr;

use axum::{
    extract::State,
    http::StatusCode,
    response::Html,
    routing::{get},
    Json, Router,
};
use chrono::NaiveTime;
use log::info;
use tokio::{net::TcpListener, task::JoinHandle};
use serde::{Deserialize, Serialize};

use crate::control::{ControlHandle, ControlSettings, ControlSettingsPatch};

#[derive(Clone)]
struct AppState {
    control: ControlHandle,
}

#[derive(Debug, Serialize)]
struct ControlSettingsDto {
    sunrise_start: String,
    day_start: String,
    sunset_start: String,
    night_start: String,
    day_temp: f64,
    night_temp: f64,
    day_light_level: f64,
    night_light_level: f64,
}

impl From<ControlSettings> for ControlSettingsDto {
    fn from(settings: ControlSettings) -> Self {
        Self {
            sunrise_start: settings.sunrise_start.format("%H:%M").to_string(),
            day_start: settings.day_start.format("%H:%M").to_string(),
            sunset_start: settings.sunset_start.format("%H:%M").to_string(),
            night_start: settings.night_start.format("%H:%M").to_string(),
            day_temp: settings.day_temp,
            night_temp: settings.night_temp,
            day_light_level: settings.day_light_level,
            night_light_level: settings.nigh_light_level,
        }
    }
}

#[derive(Debug, Deserialize)]
struct ControlSettingsPatchDto {
    sunrise_start: Option<String>,
    day_start: Option<String>,
    sunset_start: Option<String>,
    night_start: Option<String>,
    day_temp: Option<f64>,
    night_temp: Option<f64>,
    day_light_level: Option<f64>,
    night_light_level: Option<f64>,
}

pub struct WebActor {
    addr: SocketAddr,
    state: AppState,
}

impl WebActor {
    pub fn spawn(control: ControlHandle) -> JoinHandle<std::io::Result<()>> {
        let addr = env_addr("WEB_BIND", "0.0.0.0:3333");
        let actor = Self {
            addr,
            state: AppState { control },
        };

        tokio::spawn(actor.run())
    }

    async fn run(self) -> std::io::Result<()> {
        let app = Router::new()
            .route("/", get(index))
            .route("/api/settings", get(get_settings).post(set_settings))
            .with_state(self.state);

        let listener = TcpListener::bind(self.addr).await?;
        info!("Web UI listening on http://{}", self.addr);
        axum::serve(listener, app).await
    }
}

async fn index() -> Html<&'static str> {
    Html(INDEX_HTML)
}

async fn get_settings(State(state): State<AppState>) -> Result<Json<ControlSettingsDto>, StatusCode> {
    let Some(settings) = state.control.get_settings().await else {
        return Err(StatusCode::SERVICE_UNAVAILABLE);
    };
    Ok(Json(settings.into()))
}

async fn set_settings(
    State(state): State<AppState>,
    Json(patch): Json<ControlSettingsPatchDto>,
) -> Result<StatusCode, (StatusCode, String)> {
    let parsed = parse_patch(patch)?;
    state
        .control
        .update_settings(parsed)
        .map_err(|_| (StatusCode::SERVICE_UNAVAILABLE, "control actor unavailable".to_string()))?;
    Ok(StatusCode::NO_CONTENT)
}

fn parse_patch(dto: ControlSettingsPatchDto) -> Result<ControlSettingsPatch, (StatusCode, String)> {
    let sunrise_start = match dto.sunrise_start {
        None => None,
        Some(s) => Some(parse_time(&s).map_err(|e| (StatusCode::BAD_REQUEST, e))?),
    };
    let day_start = match dto.day_start {
        None => None,
        Some(s) => Some(parse_time(&s).map_err(|e| (StatusCode::BAD_REQUEST, e))?),
    };
    let sunset_start = match dto.sunset_start {
        None => None,
        Some(s) => Some(parse_time(&s).map_err(|e| (StatusCode::BAD_REQUEST, e))?),
    };
    let night_start = match dto.night_start {
        None => None,
        Some(s) => Some(parse_time(&s).map_err(|e| (StatusCode::BAD_REQUEST, e))?),
    };

    if let Some(v) = dto.day_light_level 
        && !(0.0..=1.0).contains(&v) {
        return Err((StatusCode::BAD_REQUEST, "day_light_level must be between 0.0 and 1.0".to_string()));
        
    }

    if let Some(v) = dto.night_light_level 
        && !(0.0..=1.0).contains(&v) {
        return Err((StatusCode::BAD_REQUEST, "night_light_level must be between 0.0 and 1.0".to_string()));
    }

    Ok(ControlSettingsPatch {
        sunrise_start,
        day_start,
        sunset_start,
        night_start,
        day_temp: dto.day_temp,
        night_temp: dto.night_temp,
        day_light_level: dto.day_light_level,
        nigh_light_level: dto.night_light_level,
    })
}

fn parse_time(s: &str) -> Result<NaiveTime, String> {
    NaiveTime::parse_from_str(s.trim(), "%H:%M")
        .or_else(|_| NaiveTime::parse_from_str(s.trim(), "%H:%M:%S"))
        .map_err(|_| format!("invalid time format: '{s}' (expected HH:MM or HH:MM:SS)"))
}

fn env_addr(key: &str, default: &str) -> SocketAddr {
    let raw = std::env::var(key).unwrap_or_else(|_| default.to_string());
    raw.parse::<SocketAddr>().unwrap_or_else(|_| default.parse().expect("default addr"))
}

const INDEX_HTML: &str = include_str!("web/index.html");