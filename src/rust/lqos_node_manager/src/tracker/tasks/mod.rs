mod cpu;
mod devices;
mod disk;
mod dns;
mod ram;
mod throughput;

use cpu::Cpu;
use devices::Devices;
use disk::Disk;
use dns::Dns;
use ram::Ram;
use throughput::Throughput;

use enum_dispatch::enum_dispatch;

#[enum_dispatch]
trait Task {
    fn cacheable(&self) -> bool;
    fn execute(&self) -> TaskResult;
    fn key(&self) -> String;
}

#[enum_dispatch(Task)]
pub enum Tasks {
    Cpu,
    Devices,
    Disk,
    Dns,
    Ram,
    Throughput,
}