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
use tokio::io::{AsyncWriteExt, WriteHalf};
use tokio::sync::mpsc;
use tokio_serial::SerialStream;

#[derive(Debug)]
pub enum SenderMsg {
    /// sets the target for the heating control in °C 
    TargetTemperature(f32),
    /// 0.0 is off and 1.0 is maximum brightness
    LEDBrightness(f32),
}

impl SenderMsg {
    fn to_line(&self) -> String {
        match *self {
            SenderMsg::TargetTemperature(value) => format!("target-temperature {value:.2}\n"),
            SenderMsg::LEDBrightness(value) => format!("led-brightness {value:.2}\n"),
        }
    }
}

#[derive(Clone)]
pub struct SenderHandle {
    tx: mpsc::UnboundedSender<SenderMsg>,
}

impl SenderHandle {
    pub fn send_command(&self, message: SenderMsg) -> Result<(), tokio::sync::mpsc::error::SendError<SenderMsg>> {
        self.tx.send(message)
    }
}

pub struct SenderActor {
    rx: mpsc::UnboundedReceiver<SenderMsg>,
    write_half: Option<WriteHalf<SerialStream>>,
}

impl SenderActor {
    pub fn spawn(write_half: Option<WriteHalf<SerialStream>>) -> SenderHandle {
        let (tx, rx) = mpsc::unbounded_channel();

        let actor = Self { rx, write_half };
        tokio::spawn(actor.run());

        SenderHandle { tx }
    }

    async fn run(mut self) {
        while let Some(msg) = self.rx.recv().await {
            let line = msg.to_line();

            match self.write_half {
                Some(ref mut writer) => {
                    if let Err(err) = writer.write_all(line.as_bytes()).await {
                        warn!("Failed to write to serial port: {err}");
                        continue;
                    }

                    if let Err(err) = writer.flush().await {
                        warn!("Failed to flush serial port: {err}");
                        continue;
                    }
                }
                None => {}
            };
            

            debug!("Send Command: {}", line.trim_end_matches(&['\r', '\n'][..]));
        }
    }
}