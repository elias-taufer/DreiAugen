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


use tokio::sync::{mpsc, oneshot};
use tokio::fs;
use log::{warn};

use crate::control::{ControlSettingsPatch};
pub enum  PersistanceMsg {
    ReadSettings(oneshot::Sender<ControlSettingsPatch>),
    WriteSettings(ControlSettingsPatch),
}

#[derive(Clone)]
pub struct PersistanceHandle {
    tx: mpsc::UnboundedSender<PersistanceMsg>,
}

impl PersistanceHandle {
    pub fn new(tx: mpsc::UnboundedSender<PersistanceMsg>) -> Self {
        Self { tx }
    }

    /**
     *  Simply reads the settings from the specified config file.
     */
    pub async fn read_settings(&self) -> Option<ControlSettingsPatch> {
        let (tx, rx) = oneshot::channel();
        let _ = self.tx.send(PersistanceMsg::ReadSettings(tx));
        rx.await.ok()
    }

    /**
     *  Writes the patch to the specified config file.
     */
    pub async fn write_settings(&self, patch: ControlSettingsPatch) 
        -> Result<(), mpsc::error::SendError<PersistanceMsg>> {
        self.tx.send(PersistanceMsg::WriteSettings(patch))
    }
}

/**
 * Actor for handling the persistance of the config.
 * Uses serde for serialization of the data.
 */
pub struct PersistanceActor {
    rx: mpsc::UnboundedReceiver<PersistanceMsg>,
    config_path: String,
}

impl PersistanceActor {
    pub fn spawn(rx: mpsc::UnboundedReceiver<PersistanceMsg>, config_path: String) {

        let actor = Self {
            rx,
            config_path,
        };

        tokio::spawn(actor.run());
    }

    async fn run(mut self) {
        while let Some(msg) = self.rx.recv().await {
            match msg {
                PersistanceMsg::ReadSettings(tx) => {
                    let settings = match fs::read_to_string(&self.config_path).await {
                        Ok(content) => {
                            match serde_json::from_str::<ControlSettingsPatch>(&content) {
                                Ok(settings) => settings,
                                Err(e) => {
                                    warn!("Could not parse settings from content of config file: {}", e);
                                    ControlSettingsPatch::default()
                                }
                            }
                        }
                        Err(e) => {
                            warn!("Failed to read settings from the config file: {}", e);
                            // Just return a empty ControlSettingsPatch struct on error
                            // to not disrupt runtime
                            ControlSettingsPatch::default()
                        },
                    };

                    let _ = tx.send(settings);
                }
                PersistanceMsg::WriteSettings(settings) => {
                    match serde_json::to_vec_pretty(&settings) {
                        Ok(json) => {
                            if let Err(err) = fs::write(&self.config_path, json).await {
                                warn!("failed to write config: {err}");
                            }
                        }
                        Err(err) => {
                            warn!("failed to serialize settings: {err}");
                        }
                    }
                }
            }
        }
    }
}