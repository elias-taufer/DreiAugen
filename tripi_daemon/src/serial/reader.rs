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

use std::time::Duration;
use log::{debug, warn};
use thiserror::Error;
use tokio::io::{AsyncBufReadExt, BufReader, ReadHalf};
use tokio::task::JoinHandle;
use tokio::time;
use tokio_serial::SerialStream;

use crate::control::{ControlHandle};
use crate::influx::InfluxHandle;

#[derive(Debug)]
pub struct Reading {
    pub sensor_id: String,
    pub sensor_type: String,
    pub value: f64,
}

impl Reading {
    pub fn from_key_value_line(line: &str) -> Result<Self, ParseReadingError> {
        let mut sensor_id: Option<String> = None;
        let mut sensor_type: Option<String> = None;
        let mut value: Option<f64> = None;

        for part in line.split_whitespace() {
            let Some((k, v)) = part.split_once('=') else {
                debug!("Token not implemented: {part}");
                continue; // ignore unimplemented tokens
            };

            match k {
                "sensor_id" => sensor_id = Some(v.to_owned()),
                "sensor_type" => sensor_type = Some(v.to_owned()),
                "value" => value = v.parse::<f64>().ok(),
                _ => {}
            }
        }

        Ok(Self {
            sensor_id: sensor_id.ok_or(ParseReadingError::MissingSensorId)?,
            sensor_type: sensor_type.ok_or(ParseReadingError::MissingSensorType)?,
            value: value.ok_or(ParseReadingError::MissingOrBadValue)?,
        })
    }
}

#[derive(Error, Debug)]
pub enum ParseReadingError {
    #[error("missing sensor_id")]
    MissingSensorId,
    #[error("missing sensor_type")]
    MissingSensorType,
    #[error("missing or invalid value")]
    MissingOrBadValue,
}

pub struct ReaderActor {
    read_half: Option<ReadHalf<SerialStream>>,
    read_timeout: Duration,

    _control: ControlHandle,
    influx: InfluxHandle,
}

impl ReaderActor {
    pub fn spawn(
        read_half: Option<ReadHalf<SerialStream>>,
        control: ControlHandle,
        influx: InfluxHandle,
        read_timeout: Duration,
    ) -> JoinHandle<std::io::Result<()>> {
        let actor = Self {
            read_half,
            read_timeout,
            _control: control,
            influx,
        };

        tokio::spawn(actor.run())
    }

    async fn run(mut self) -> std::io::Result<()> {

        match self.read_half {
            Some(ref mut read_half) => {
                let mut reader = BufReader::new(read_half);
                loop {
                    let mut line = String::new();

                    let read_res = time::timeout(self.read_timeout, reader.read_line(&mut line)).await;

                    let n = match read_res {
                        Err(_) => {
                            continue;
                        }
                        Ok(Ok(n)) => n,
                        Ok(Err(err)) => {
                            warn!("Unable to read line from serial port: {err}");
                            continue;
                        }
                    };

                    if n == 0 {
                        continue; // no data 
                    }

                    let line = line.trim_end_matches(&['\r', '\n'][..]).to_string();
                    if line.is_empty() {
                        continue;
                    }

                    debug!("Read from serial port: {line}");

                    // 2) Parse the subset that represents sensor readings and forward to influx
                    let reading = match Reading::from_key_value_line(&line) {
                        Ok(r) => r,
                        Err(err) => {
                            warn!("Could not parse reading from serial port: {err}");
                            continue;
                        }
                    };

                    let _ = self.influx.send_reading(reading);
                }
            },
            None => {
                let mut tick = time::interval(std::time::Duration::from_secs(1));
                loop {
                    tokio::select! {
                        _ = tick.tick() => {}
                    }
                }
            },
        }
        
    }
}
