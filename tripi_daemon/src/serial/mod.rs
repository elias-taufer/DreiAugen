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

use log::warn;
use tokio::sync::mpsc;
use tokio_serial::{SerialPortBuilderExt};
use tokio::time::{self};

use crate::serial::sender::{SenderHandle, SenderMsg};
use crate::serial::reader::{ReaderHandle, ReaderMsg};

pub mod reader;
pub mod sender;

pub enum SerialState {
    Connected,
    Disconnected,
    Reconnecting,
}

#[derive(Debug)]
pub enum SerialManagerMsg {
    SerialPortReadFail,
    SerialPortWriteFail,
}

#[derive(Clone)]
pub struct SerialManagerHandle {
    tx: mpsc::UnboundedSender<SerialManagerMsg>,
}

impl SerialManagerHandle {
    pub fn new(tx: mpsc::UnboundedSender<SerialManagerMsg>) -> Self {
        Self { tx }
    }

    pub fn send(&self, message: SerialManagerMsg) -> Result<(), tokio::sync::mpsc::error::SendError<SerialManagerMsg>> {
        self.tx.send(message)
    }
}

/**
 * Handles the serial port. When a failure while reading or writing on the serial port is detected
   the SerialManagerActor will try to open the port again and provide the read and write halfes to 
   their corresponding actors ReaderActor and SenderActor.
 * This depends on if the SenderActor was able to execute WriteHalf::shutdown previousle. 
   If not, a restart of the application is probably necessary to recover.
 */
pub struct SerialManagerActor {
    state: SerialState,
    sender: SenderHandle,
    reader: ReaderHandle, 
    rx: mpsc::UnboundedReceiver<SerialManagerMsg>,
    device_path: String,
    baud_rate: u32,
}

impl SerialManagerActor {
    pub fn spawn(
        rx: mpsc::UnboundedReceiver<SerialManagerMsg>,
        sender: SenderHandle,
        reader: ReaderHandle,
        device_path: String,
        baud_rate: u32,
    ) {
        let state = SerialState::Disconnected;
        

        let actor = Self {
            state,
            sender,
            reader,
            rx,
            device_path,
            baud_rate,
        };

        tokio::spawn(actor.run());
    }

    /// Runs the Actor.
    /// 
    /// Checks if the Serial port is open on every tick (2 sec intervall).
    /// If the Serial Port is not open it will try to open it.
    async fn run(mut self) {

        let mut tick = time::interval(std::time::Duration::from_secs(2));

        loop {
            tokio::select! {
                msg = self.rx.recv() => {
                    match msg {
                        Some(msg) => self.handle_msg(msg).await,
                        None => break, 
                    }
                }
                _ = tick.tick() => {
                    self.on_tick().await;
                }
            }
        }
    }

    async fn on_tick(&mut self) {
        if matches!(self.state, SerialState::Disconnected) {
            self.connect().await;
        }
    }

    /// Tries to open the serial port and sends the SenderActor and ReaderActor their
    /// corresponding write-/read halfes.
    async fn connect(&mut self) {
        self.state = SerialState::Reconnecting;

        let port = match tokio_serial::new(self.device_path.clone(), self.baud_rate).open_native_async() {
            Ok(port) => { port }
            Err(e) => {
                self.state = SerialState::Disconnected;
                warn!("Failed to connect to serial port: {}", e);
                return;
            }
        };

        let (read_half, write_half) = tokio::io::split(port);

        self.state = SerialState::Connected;

        let _ = self.reader.send(ReaderMsg::SetReader(read_half));
        let _ = self.sender.send_command(SenderMsg::SetWriter(write_half));
    }

    async fn handle_msg(&mut self, msg: SerialManagerMsg) {
        match msg {
            SerialManagerMsg::SerialPortReadFail => {
                self.state = SerialState::Disconnected;
                let _ = self.sender.send_command(SenderMsg::Disconnect);
                let _ = self.reader.send(ReaderMsg::Disconnect);
            },
            SerialManagerMsg::SerialPortWriteFail => {
                self.state = SerialState::Disconnected;
                let _ = self.reader.send(ReaderMsg::Disconnect);
                let _ = self.sender.send_command(SenderMsg::Disconnect);
            }
        }
    }
}