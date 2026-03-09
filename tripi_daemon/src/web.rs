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
use serde::{Deserialize, Serialize};
use tokio::{net::TcpListener, task::JoinHandle};

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

    if let Some(v) = dto.day_light_level {
        if !(0.0..=1.0).contains(&v) {
            return Err((StatusCode::BAD_REQUEST, "day_light_level must be between 0.0 and 1.0".to_string()));
        }
    }

    if let Some(v) = dto.night_light_level {
        if !(0.0..=1.0).contains(&v) {
            return Err((StatusCode::BAD_REQUEST, "night_light_level must be between 0.0 and 1.0".to_string()));
        }
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

const INDEX_HTML: &str = r#"<!doctype html>
<html lang="de">
  <head>
    <meta charset="utf-8" />
    <meta name="viewport" content="width=device-width, initial-scale=1" />
    <title>tripi_daemon - Einstellungen</title>
    <style>
      body { font-family: system-ui, -apple-system, Segoe UI, Roboto, sans-serif; max-width: 720px; margin: 24px auto; padding: 0 12px; }
      h1 { font-size: 20px; margin: 0 0 16px; }
      form { display: grid; grid-template-columns: 1fr 1fr; gap: 12px 16px; }
      label { display: grid; gap: 6px; font-size: 13px; }
      input { padding: 8px; font-size: 14px; }
      .row { grid-column: 1 / -1; display: flex; gap: 12px; align-items: center; }
      button { padding: 8px 12px; font-size: 14px; cursor: pointer; }
      code { background: #f4f4f4; padding: 2px 6px; border-radius: 6px; }
      #msg { font-size: 13px; }
    </style>
  </head>
  <body>
    <h1>Einstellungen</h1>
    <p>API: <code>/api/settings</code></p>

    <form id="f">
      <label>Sunrise Start (HH:MM)
        <input id="sunrise_start" placeholder="08:00" />
      </label>
      <label>Day Start (HH:MM)
        <input id="day_start" placeholder="07:00" />
      </label>
      <label>Sunset Start (HH:MM)
        <input id="sunset_start" placeholder="22:30" />
      </label>
      <label>Night Start (HH:MM)
        <input id="night_start" placeholder="23:30" />
      </label>
      <label>Day Temp (°C)
        <input id="day_temp" type="number" step="0.1" />
      </label>
      <label>Night Temp (°C)
        <input id="night_temp" type="number" step="0.1" />
      </label>
      <label>Day Light Level (0.0-1.0)
        <input id="day_light_level" type="number" min="0" max="1" step="0.01" />
      </label>
      <label>Night Light Level (0.0-1.0)
        <input id="night_light_level" type="number" min="0" max="1" step="0.01" />
      </label>

      <div class="row">
        <button type="submit">Speichern</button>
        <button type="button" id="reload">Neu laden</button>
        <span id="msg"></span>
      </div>
    </form>

    <script>
      const el = (id) => document.getElementById(id);
      const msg = (t) => { el('msg').textContent = t; };

      async function load() {
        msg('');
        const r = await fetch('/api/settings');
        if (!r.ok) { msg('Fehler beim Laden: ' + r.status); return; }
        const s = await r.json();
        el('sunrise_start').value = s.sunrise_start;
        el('day_start').value = s.day_start;
        el('sunset_start').value = s.sunset_start;
        el('night_start').value = s.night_start;
        el('day_temp').value = s.day_temp;
        el('night_temp').value = s.night_temp;
        el('day_light_level').value = s.day_light_level;
        el('night_light_level').value = s.night_light_level;
      }

      async function save(e) {
        e.preventDefault();
        msg('');

        const payload = {
          sunrise_start: el('sunrise_start').value,
          day_start: el('day_start').value,
          sunset_start: el('sunset_start').value,
          night_start: el('night_start').value,
          day_temp: Number(el('day_temp').value),
          night_temp: Number(el('night_temp').value),
          day_light_level: Number(el('day_light_level').value),
          night_light_level: Number(el('night_light_level').value),
        };

        const r = await fetch('/api/settings', {
          method: 'POST',
          headers: { 'content-type': 'application/json' },
          body: JSON.stringify(payload),
        });

        if (r.status === 204) { msg('Gespeichert.'); return; }
        const t = await r.text();
        msg('Fehler: ' + r.status + ' ' + t);
      }

      el('f').addEventListener('submit', save);
      el('reload').addEventListener('click', load);
      load();
    </script>
  </body>
</html>"#;