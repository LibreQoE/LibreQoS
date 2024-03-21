# Git install

## Clone the repo

The recommended install location is `/opt/libreqos`
Go to the install location, and clone the repo:

```shell
cd /opt/
git clone https://github.com/LibreQoE/LibreQoS.git libreqos
sudo chown -R YOUR_USER /opt/libreqos
cd /opt/libreqos/
git switch develop
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
pip install requirements.txt --break-system-packages
sudo pip install requirements.txt --break-system-packages
```

## Python 3.10 quirk (will fix later)
```
cd /opt/libreqos/src/rust
cargo update
sudo cp /usr/lib/x86_64-linux-gnu/libpython3.11.so /usr/lib/x86_64-linux-gnu/libpython3.10.so.1.0
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
