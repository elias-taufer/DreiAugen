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

use crate::serial::{SerialManagerHandle, SerialManagerMsg};

#[derive(Debug)]
pub enum SenderMsg {
    /// sets the target for the heating control in °C 
    TargetTemperature(f32),
    /// 0.0 is off and 1.0 is maximum brightness
    LEDBrightness(f32),

    SetWriter(WriteHalf<SerialStream>),

    Disconnect,
}

impl SenderMsg {
    fn to_line(&self) -> String {
        match self {
            SenderMsg::TargetTemperature(value) => format!("target-temperature {value:.2}\n"),
            SenderMsg::LEDBrightness(value) => format!("led-brightness {value:.2}\n"),
            SenderMsg::SetWriter(_writer) => "Replace Writer".to_string(),
            SenderMsg::Disconnect => "disconnect".to_string(),
        }
    }
}

#[derive(Clone)]
pub struct SenderHandle {
    tx: mpsc::UnboundedSender<SenderMsg>,
}

impl SenderHandle {
    pub fn new(tx: mpsc::UnboundedSender<SenderMsg>) -> Self {
        Self { tx }
    }

    pub fn send_command(&self, message: SenderMsg) -> Result<(), tokio::sync::mpsc::error::SendError<SenderMsg>> {
        self.tx.send(message)
    }
}

pub enum SenderState {
    Disconnected,
    Connected(WriteHalf<SerialStream>),
}

/// Will send the received data over the serial port.
/// 
/// Will try to shut down the serial connection it currently holds 
/// when Disconnect Message is received to free the connection 
/// for a reconnect.
pub struct SenderActor {
    state: SenderState,
    rx: mpsc::UnboundedReceiver<SenderMsg>,

    serial_manager: SerialManagerHandle,
}

impl SenderActor {
    pub fn spawn(rx_sender: mpsc::UnboundedReceiver<SenderMsg>, serial_manager: SerialManagerHandle) {

        let state = SenderState::Disconnected;

        let actor = Self { state, rx: rx_sender, serial_manager };
        tokio::spawn(actor.run());
    }

    /// Runs the actor and closes the write_half if message for Disconnect is received.
    async fn run(mut self) {
        while let Some(msg) = self.rx.recv().await {

            match msg {
                SenderMsg::Disconnect => { 
                    if let SenderState::Connected(write_half) = &mut self.state {
                        let _ = write_half.shutdown().await;
                    }
                    self.state = SenderState::Disconnected 
                },
                SenderMsg::SetWriter(writer) => self.state = SenderState::Connected(writer),
                _ => Self::send_msg_to_serial(msg.to_line(), &mut self.state, &self.serial_manager).await,
            }

        }
    }

    async fn send_msg_to_serial(message: String, state: &mut SenderState, serial_manager: &SerialManagerHandle) {
        if let SenderState::Connected(writer) = state {
            if let Err(err) = writer.write_all(message.as_bytes()).await {
                let _ = serial_manager.send(SerialManagerMsg::SerialPortWriteFail);
                warn!("Failed to write to serial port: {err}");
                return;
            }

            if let Err(err) = writer.flush().await {
                let _ = serial_manager.send(SerialManagerMsg::SerialPortWriteFail);
                warn!("Failed to flush serial port: {err}");
                return;
            }

            debug!("Send Command: {}", message.trim_end_matches(&['\r', '\n'][..]));
        }
    }
}