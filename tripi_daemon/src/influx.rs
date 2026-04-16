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

use chrono::Local;
use futures::prelude::stream;
use influxdb2::Client;
use influxdb2_derive::WriteDataPoint;
use log::{debug, warn};
use tokio::sync::mpsc;

use crate::serial::reader::Reading;

#[derive(Default, WriteDataPoint)]
#[measurement = "pid_value"]
struct PidValue {
    #[influxdb(tag)]
    pid_id: String,
    #[influxdb(tag)]
    sensor_type: String,
    #[influxdb(tag)]
    device_id: String,
    #[influxdb(field)]
    value: f64,
    #[influxdb(timestamp)]
    time: i64,
}

#[derive(Default, WriteDataPoint)]
#[measurement = "water_temperature"]
struct WaterTemperature {
    #[influxdb(tag)]
    sensor_id: String,
    #[influxdb(tag)]
    sensor_type: String,
    #[influxdb(tag)]
    device_id: String,
    #[influxdb(field)]
    temperature_c: f64,
    #[influxdb(timestamp)]
    time: i64,
}

#[derive(Default, WriteDataPoint)]
#[measurement = "brightness"]
struct Brightness {
    #[influxdb(tag)]
    sensor_type: String,
    #[influxdb(tag)]
    device_id: String,
    #[influxdb(field)]
    value: f64,
    #[influxdb(timestamp)]
    time: i64,
}

#[derive(Clone, Debug)]
pub struct InfluxConfig {
    pub host: String,
    pub org: String,
    pub token: String,
    pub bucket: String,
    pub device_id: String,
}

impl InfluxConfig {
    pub fn from_env() -> Self {
        let host = std::env::var("INFLUXDB_URL").expect("Unable to find INFLUXDB_URL");
        let org = std::env::var("INFLUXDB_ORG").expect("Unable to find INFLUXDB_ORG");
        let token = std::env::var("INFLUXDB_TOKEN").expect("Unable to find INFLUXDB_TOKEN");
        let bucket = std::env::var("INFLUXDB_BUCKET").expect("Unable to find INFLUXDB_BUCKET");
        let device_id = std::env::var("DEVICE_ID").unwrap_or_else(|_| "tripi".to_owned());

        Self {
            host,
            org,
            token,
            bucket,
            device_id,
        }
    }
}

#[derive(Clone)]
pub struct InfluxHandle {
    tx: mpsc::UnboundedSender<Reading>,
}

impl InfluxHandle {
    pub fn send_reading(
        &self,
        reading: Reading,
    ) -> Result<(), mpsc::error::SendError<Reading>> {
        self.tx.send(reading)
    }
}

pub struct InfluxActor {
    rx: mpsc::UnboundedReceiver<Reading>,
    client: Client,
    bucket: String,
    device_id: String,
}

impl InfluxActor {
    pub fn spawn(cfg: InfluxConfig) -> InfluxHandle {
        let (tx, rx) = mpsc::unbounded_channel();

        let actor = Self {
            rx,
            client: Client::new(cfg.host, cfg.org, cfg.token),
            bucket: cfg.bucket,
            device_id: cfg.device_id,
        };

        tokio::spawn(actor.run());

        InfluxHandle { tx }
    }

    async fn run(mut self) {
        while let Some(reading) = self.rx.recv().await {
            match reading.sensor_type.as_str() {
                "ds18b20" => {
                    if let Err(err) = self.write_water_temperature(reading).await {
                        warn!("Influx write failed: {err}");
                    }
                }
                "internal_pid_val" => {
                    if let Err(err) = self.write_pid_val(reading).await {
                        warn!("Influx write failed: {err}");
                    }
                }
                "light" => {
                    if let Err(err) = self.write_brightness(reading).await {
                        warn!("Influx write failed: {err}");
                    }
                }
                "err" => {
                    warn!("Error from esp: {reading:?}");
                }
                "dbg" => {
                    debug!("Debug from esp: {reading:?}");
                }
                _ => {
                    warn!("Unknown sensor type: {}", reading.sensor_type);
                }
            }
        }
    }

    async fn write_pid_val(&self, reading: Reading) -> Result<(), influxdb2::RequestError> {
        let point = PidValue {
            pid_id: reading.sensor_id,
            sensor_type: reading.sensor_type,
            device_id: self.device_id.clone(),
            value: reading.value,
            time: Local::now()
                .timestamp_nanos_opt()
                .expect("no nanos timestamp available"),
        };

        self.client.write(&self.bucket, stream::iter([point])).await
    }

    async fn write_water_temperature(&self, reading: Reading) -> Result<(), influxdb2::RequestError> {
        let point = WaterTemperature {
            sensor_id: reading.sensor_id,
            sensor_type: reading.sensor_type,
            device_id: self.device_id.clone(),
            temperature_c: reading.value,
            time: Local::now()
                .timestamp_nanos_opt()
                .expect("no nanos timestamp available"),
        };

        self.client.write(&self.bucket, stream::iter([point])).await
    }

    async fn write_brightness(&self, reading: Reading) -> Result<(), influxdb2::RequestError> {
        let point = Brightness {
            sensor_type: reading.sensor_type,
            device_id: self.device_id.clone(),
            value: reading.value,
            time: Local::now()
                .timestamp_nanos_opt()
                .expect("no nanos timestamp available"),
        };

        self.client.write(&self.bucket, stream::iter([point])).await
    }
}
