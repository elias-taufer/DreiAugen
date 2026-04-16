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

use log::{debug, warn};
use std::time::Duration;
use thiserror::Error;
use tokio::io::{AsyncBufReadExt, BufReader, ReadHalf};
use tokio::sync::mpsc;
use tokio::time;
use tokio_serial::SerialStream;

use crate::control::ControlHandle;
use crate::influx::InfluxHandle;
use crate::serial::{SerialManagerHandle, SerialManagerMsg};

#[derive(Debug)]
pub struct Reading {
    pub sensor_id: String,
    pub sensor_type: String,
    pub value: f64,
}

impl Reading {
    pub fn from_key_value_line(line: &str) -> Result<Self, ValueMissingError> {
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
            sensor_id: sensor_id.ok_or(ValueMissingError::SensorId)?,
            sensor_type: sensor_type.ok_or(ValueMissingError::SensorType)?,
            value: value.ok_or(ValueMissingError::MissingOrBadValue)?,
        })
    }
}

#[derive(Error, Debug)]
pub enum ValueMissingError {
    #[error("missing sensor_id")]
    SensorId,
    #[error("missing sensor_type")]
    SensorType,
    #[error("missing or invalid value")]
    MissingOrBadValue,
}

pub enum ReaderMsg {
    SetReader(ReadHalf<SerialStream>),
    Disconnect,
}

#[derive(Clone)]
pub struct ReaderHandle {
    tx: mpsc::UnboundedSender<ReaderMsg>,
}

impl ReaderHandle {
    pub fn new(tx: mpsc::UnboundedSender<ReaderMsg>) -> Self {
        Self { tx }
    }

    pub fn send(&self, message: ReaderMsg) -> Result<(), tokio::sync::mpsc::error::SendError<ReaderMsg>> {
        self.tx.send(message)
    }
}

pub enum ReaderState {
    Disconnected,
    Connected(BufReader<ReadHalf<SerialStream>>),
}

pub struct ReaderActor {
    rx: mpsc::UnboundedReceiver<ReaderMsg>,
    state: ReaderState,
    read_timeout: Duration,

    serial_manager: SerialManagerHandle,
    _control: ControlHandle,
    influx: InfluxHandle,
}

impl ReaderActor {
    pub fn spawn(
        rx: mpsc::UnboundedReceiver<ReaderMsg>,
        serial_manager: SerialManagerHandle,
        control: ControlHandle,
        influx: InfluxHandle,
        read_timeout: Duration,
    ) {
        let state = ReaderState::Disconnected;

        let actor = Self {
            rx,
            state,
            read_timeout,
            serial_manager,
            _control: control,
            influx,
        };

        tokio::spawn(actor.run());
    }

    async fn handle_msg(&mut self, msg: ReaderMsg) {
        match msg {
            ReaderMsg::SetReader(reader) => {
                self.state = ReaderState::Connected(BufReader::new(reader));
            }
            ReaderMsg::Disconnect => {
                self.state = ReaderState::Disconnected;
            }
        }
    }
 
    async fn read(reader: &mut BufReader<ReadHalf<SerialStream>>, read_timeout: Duration) 
        -> tokio::io::Result<Option<Reading>> 
    {
        let mut line = String::new();

        let result = time::timeout(read_timeout, reader.read_line(&mut line)).await;
         
        match result {
            Ok(Ok(0)) => Ok(None), // no data 
            Ok(Ok(_)) => {
                let line = line.trim_end_matches(&['\r', '\n'][..]).to_string();
                if line.is_empty() {
                    return Ok(None);
                }

                debug!("Read from serial port: {line}");

                // 2) Parse the subset that represents sensor readings and forward to influx
                match Reading::from_key_value_line(&line) {
                    Ok(reading) => Ok(Some(reading)),
                    Err(err) => {
                        warn!("Could not parse reading from serial port: {err}");
                        Ok(None)
                    }
                }
            }
            Ok(Err(e)) => Err(e),
            Err(_) => 
                // Timeout occurred, no line ready. Expected behaviour.
                Ok(None)
            
        }
                        
    }

    async fn run(mut self) {
        loop {

            match &mut self.state {
                ReaderState::Connected(reader) => {
                    tokio::select! {
                        msg = self.rx.recv() => {
                            match msg {
                                Some(msg) => self.handle_msg(msg).await,
                                None => break, // channel closed
                            }
                        }

                        result =  Self::read(reader, self.read_timeout) => {
                            match result {
                                Ok(Some(reading)) => {
                                    let _ = self.influx.send_reading(reading);
                                }
                                Ok(None) => {
                                    // timeout, empty line, or no data
                                }
                                Err(e) => {
                                    eprintln!("Serial read error: {e}");
                                    
                                    let _ = self.serial_manager.send(SerialManagerMsg::SerialPortReadFail);
                                }
                            }
                        }
                        
                    }
                }

                ReaderState::Disconnected => {
                    // Only handle messages when disconnected
                    if let Some(msg) = self.rx.recv().await {
                        self.handle_msg(msg).await;
                    } else {
                        break;
                    }
                }
            }

        }

        
    }
}
