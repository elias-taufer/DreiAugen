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

use tokio_serial::{SerialPortBuilderExt, SerialStream};
use tokio::io::{ReadHalf, WriteHalf};
use std::time::Duration;
use std::{
    env,
    fs,
    io,
    path::{Path, PathBuf},
};

mod control;
mod influx;
mod serial {
    pub mod sender;
    pub mod reader;
}
mod web;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Load cfg before Tokio runtime exists
    let cfg_path = config_path_next_to_exe("tripi_daemon.cfg")?;

    load_env_from_cfg_file(&cfg_path)?;

    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;

    rt.block_on(async_main())
}

fn config_path_next_to_exe(filename: &str) -> io::Result<PathBuf> {
    let exe = env::current_exe()?;
    let dir = exe.parent().unwrap_or_else(|| Path::new("."));
    Ok(dir.join(filename))
}

fn load_env_from_cfg_file(path: &Path) -> io::Result<()> {
    let text = fs::read_to_string(path)?;
    for raw in text.lines() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let Some((k, v)) = line.split_once('=') else {
            continue;
        };

        let key = k.trim();
        let val = v.trim();

        if env::var_os(key).is_none() {
            // Safe here because this runs before Tokio runtime threads exist.
            unsafe {
                env::set_var(key, val);
            }
        }
    }
    Ok(())
}

async fn async_main() -> Result<(), Box<dyn std::error::Error>> {
    env_logger::init();

    // load settings

    let serial_path = std::env::var("SERIAL_PORT")
        .expect("Unable to find SERIAL_PORT");
    let baud_rate: u32 = std::env::var("SERIAL_BAUD")
        .expect("Unable to find SERIAL_BAUD")
        .trim()
        .parse()
        .expect("Unable to read SERIAL_BAUD");
    let serial_timeout: u64 = std::env::var("SERIAL_TIMEOUT")
        .expect("Unable to find SERIAL_TIMEOUT")
        .trim()
        .parse()
        .expect("Unable to read SERIAL_TIMEOUT");

    let influx_cfg = influx::InfluxConfig::from_env();

    // open serial once, then split read/write
    // prevents trying to open the same serial device twice
    let (read_half, write_half): (Option<ReadHalf<SerialStream>>, Option<WriteHalf<SerialStream>>) =
    match tokio_serial::new(serial_path, baud_rate).open_native_async() {
        Ok(port) => {
            let (r, w) = tokio::io::split(port);
            (Some(r), Some(w))
        }
        Err(_e) => {
            // loggen, wenn du willst
            (None, None)
        }
    };

    // Spawn Actors
    let sender_handle = serial::sender::SenderActor::spawn(write_half);
    let influx_handle = influx::InfluxActor::spawn(influx_cfg);
    let control_handle = control::ControlActor::spawn(sender_handle.clone());
    let web_handle = web::WebActor::spawn(control_handle.clone());
    let reader = 
        serial::reader::ReaderActor::spawn(
            read_half, 
            control_handle, 
            influx_handle, 
            Duration::from_millis(serial_timeout),
        );

    tokio::select! {
        _ = tokio::signal::ctrl_c() => {
            // dropping handles closes channels and lets tasks exit naturally
        }
        res = reader => {
            // if reader ends unexpectedly, bubble up the error
            res??;
        }
        res = web_handle => {
            // if web server ends unexpectedly, bubble up the error
            res??;
        }
    }
    
    Ok(())
}
