mod cache_manager;
mod tasks;
mod thread_pool;

use tokio::time::Duration;
use tokio::time::Instant;
use sysinfo::System;

use cache_manager::CacheManager;

use tokio::sync::oneshot;
use async_channel::Sender;

use thread_pool::*;
use tasks::{Task, Tasks};
use crate::utils;

use crate::WsEvent;

pub struct Tracker {
    async_tx: Sender<WsEvent>,
    cache_manager: CacheManager,
    pool: Pool,
}

impl Tracker {
    pub fn new(async_tx: Sender<WsEvent>) -> Self {
        let cache_manager = CacheManager::default();
        let pool = Pool::new();
        Tracker {
            async_tx: async_tx,
            cache_manager: cache_manager,
            pool: pool,
        }
    }

    pub fn start(&self, interval: u16) {
        tokio::spawn(async move {
            let duration = Duration::from_millis(interval.into());
            let mut next_time = Instant::now() + duration;
            let mut sys = System::new_all();
            loop {
                let tasks = Tasks::Cpu;
                match self.trigger(task) {
                    Ok(result_rx) => {
                        self.async_tx.try_send();
                        if result_rx.result.cacheable {  }
                    },
                    Err(e) => {}
                };
                tokio::time::sleep(next_time - Instant::now()).await;
                next_time += duration;
            }
        });
    }

    pub fn trigger(&self, task: Vec<Tasks>) -> Result<oneshot::Receiver<TaskResult>, ()> {
        let (tx, rx) = oneshot::channel();
        self.pool.execute(
            if let Err(_) = tx.send(task.execute()) {
                Err("the receiver dropped");
            }
        );
        Ok(rx)
    }
}