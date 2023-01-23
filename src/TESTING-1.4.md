# How to Test V1.4

Version 1.4 is still undergoing active development, but if you'd like to benefit from it right now (or help us test/develop it!), here's a guide.

## Updating from v1.3
### Remove cron tasks from v1.3
Run ```sudo crontab -e``` and remove any entries pertaining to LibreQoS from v1.3.

## Clone the repo

> My preferred install location is `/opt/libreqos` - but you can put it wherever you want.

Go to your preferred install location, and clone the repo:

```
git clone https://github.com/LibreQoE/LibreQoS.git
```

> Now that this is in `main`, you no longer need to switch git branch. If you were previously on the `v1.4-pre-alpha-rust-integration` branch, please switch to main with `git checkout main; git pull`.

Switch to the development branch:

```
git checkout v1.4-pre-alpha-rust-integration
```

## Install Dependencies from apt and pip

You need to have a few packages from `apt` installed:

```
apt-get install -y python3-pip clang gcc gcc-multilib llvm libelf-dev git nano graphviz curl screen llvm pkg-config linux-tools-common linux-tools-`uname -r` libbpf-dev
```

Then you need to install some Python dependencies:

```
python3 -m pip install ipaddress schedule influxdb-client requests flask flask_restful flask_httpauth waitress psutil binpacking graphviz
```

## Install the Rust development system

Go to [RustUp](https://rustup.rs) and follow the instructions. Basically, run the following:

```
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
```

When Rust finishes installing, it will tell you to execute a command to place the Rust build tools into your path. You need to either execute this command or logout and back in again.

Once that's done, change directory to `/wherever_you_put_libreqos/src/`, and run:

```
./build_rust.sh
```

This will take a while the first time, but it puts everything in the right place.

## Setup the LibreQoS Daemon

Copy the daemon configuration file to `/etc`:

```
sudo cp lqos.example /etc/lqos.conf
```

Now edit the file to match your setup:

```toml
lqos_directory = '/opt/libreqos/src'
queue_check_period_ms = 1000

[tuning]
stop_irq_balance = true
netdev_budget_usecs = 8000
netdev_budget_packets = 300
rx_usecs = 8
tx_usecs = 8
disable_rxvlan = true
disable_txvlan = true
disable_offload = [ "gso", "tso", "lro", "sg", "gro" ]

interface_mapping = [
       { name = "enp1s0f1", redirect_to = "enp1s0f2", scan_vlans = false },
       { name = "enp1s0f2", redirect_to = "enp1s0f1", scan_vlans = false }
]
vlan_mapping = []
```

Change `enp1s0f1` and `enp1s0f2` to match your network interfaces. It doesn't matter which one is which.

## Configure LibreQoS

Follow the regular instructions to set your interfaces in `ispConfig.py` and your `network.json` and `ShapedDevices.csv` files.

## Configure autostart services (lqosd, lqos_node_manager)

```
cp /opt/libreqos/src/bin/lqos_node_manager.service.example /etc/systemd/system/lqos_node_manager.service
cp /opt/libreqos/src/bin/lqosd.service.example /etc/systemd/system/lqosd.service
systemctl daemon-reload
```

## Run the program (debug mode)

You can setup `lqosd` and `lqos_node_manager` as daemons to keep running (there are example `systemd` files in the `src/bin` folder). Since v1.4 is under such heavy development, I recommend using `screen` to run detached instances - and make finding issues easier.

1. Stop services: systemctl stop lqosd lqos_node_manager
2. `screen`
3. `cd /wherever_you_put_libreqos/src/bin`
4. `sudo ./lqosd`
5. Create a new `screen` window with `Ctrl-A, C`.
6. Run the webserver with `./lqos_node_manager`
7. If you didn't see errors, detach with `Ctrl-A, D`

You can now point a web browser at `http://a.b.c.d:9123` (replace `a.b.c.d` with the management IP address of your shaping server) and enjoy a real-time view of your network.

In the web browser, click `Reload LibreQoS` to setup your shaping rules.

# Updating 1.4 Once You Have It

* Note: On January 22nd 2023 /etc/lqos was changed to /etc/lqos.conf to remedy Issue #205. If upgrading, be sure to move /etc/lqos to /etc/lqos.conf

<img src="https://raw.githubusercontent.com/LibreQoE/LibreQoS/main/docs/jk.jpg" width=200px></a>

1. Resume screen with `screen -r`
2. Go to console 0 (`Ctrl-A, 0`) and stop `lqosd` with `ctrl+c`.
3. Go to console 1 (`Ctl-A, 1`) and stop `lqos_node_manager` with `ctrl+c`.
4. Detach from `screen` with `Ctrl-A, D`.
5. Change to your `LibreQoS` directory (e.g. `cd /opt/libreqos`)
6. Update from Git: `git pull`
7. Recompile: `./build-rust.sh`
8. Resume screen with `screen -r`.
9. Go to console 0 (`Ctrl-A, 0`) and run `sudo ./lqosd` to restart the bridge/manager.
10. Go to console 1 (`Ctrl-A, 1`) and run `./lqos_node_manager` to restart the web server.
11. If you didn't see errors, detach with `Ctrl-A, D` 
