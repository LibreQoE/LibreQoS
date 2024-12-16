# Git Install (For Developers Only - Not Recommended)

## Clone the repo

The recommended install location is `/opt/libreqos`
Go to the install location, and clone the repo:

```shell
cd /opt/
sudo git clone https://github.com/LibreQoE/LibreQoS.git libreqos
sudo chown -R $USER /opt/libreqos
cd /opt/libreqos/
git switch develop
git pull
```

By specifying `libreqos` at the end, git will ensure the folder name is lowercase.

## Install Dependencies from apt and pip

You need to have a few packages from `apt` installed:

```shell
sudo apt-get install -y python3-pip clang gcc gcc-multilib llvm libelf-dev git nano graphviz curl screen llvm pkg-config linux-tools-common linux-tools-`uname -r` libbpf-dev libssl-dev
```

Then you need to install some Python dependencies:

```shell
cd /opt/libreqos
PIP_BREAK_SYSTEM_PACKAGES=1 pip install -r requirements.txt
sudo PIP_BREAK_SYSTEM_PACKAGES=1 pip install -r requirements.txt
```

## Install the Rust development system

Run the following:

```shell
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
```

When Rust finishes installing, it will tell you to execute a command to place the Rust build tools into your path. You need to either execute this command or logout and back in again.

Once that's done, please run:

```shell
cd /opt/libreqos/src/
./build_rust.sh
```

This will take a while the first time, but it puts everything in the right place.

Now, to build rust crates, run:

```shell
cd rust
cargo build --all
```

## Lqos.conf

Copy the lqos.conf configuration file to `/etc`. This is not neccesarry if you installed using the .deb:

```shell
cd /opt/libreqos/src
sudo cp lqos.example /etc/lqos.conf
```

## Configuration

Proceed to configure settings [following this guide](../Quickstart/configuration.md).

## Daemon setup

## Run daemons with systemd

Note: If you used the .deb installer, you can skip this section. The .deb installer automatically sets these up.

You can now set up `lqosd`, and `lqos_scheduler` as systemd services.

```shell
sudo cp /opt/libreqos/src/bin/lqosd.service.example /etc/systemd/system/lqosd.service
sudo cp /opt/libreqos/src/bin/lqos_scheduler.service.example /etc/systemd/system/lqos_scheduler.service
```

Finally, run

```shell
sudo systemctl daemon-reload
sudo systemctl enable lqosd lqos_scheduler
sudo systemctl start lqosd lqos_scheduler
```

You can now point a web browser at `http://a.b.c.d:9123` (replace `a.b.c.d` with the management IP address of your shaping server) and enjoy a real-time view of your network.
