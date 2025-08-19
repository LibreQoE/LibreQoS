# Instalación compleja (no recomendada)

## Clonar el repositorio

La ubicación de instalación recomendada es `/opt/libreqos`
Vaya a la ubicación de instalación y clone el repositorio.:

```shell
cd /opt/
git clone https://github.com/LibreQoE/LibreQoS.git libreqos
sudo chown -R YOUR_USER /opt/libreqos
```

Al especificar `libreqos` al final, git se asegurará que el nombre de la carpeta esté en minúsculas.

## Instalar dependencias desde apt y pip

Necesitas tener algunos paquetes de `apt` instalados:

```shell
sudo apt-get install -y python3-pip clang gcc gcc-multilib llvm libelf-dev git nano graphviz curl screen llvm pkg-config linux-tools-common linux-tools-`uname -r` libbpf-dev libssl-dev
```

Luego necesitas instalar algunas dependencias de Python:

```shell
cd /opt/libreqos
python3 -m pip install -r requirements.txt
sudo python3 -m pip install -r requirements.txt
```

## Instalar el sistema de desarrollo Rust

Ve a [RustUp](https://rustup.rs) y sigue las instrucciones. Básicamente, ejecuta lo siguiente:

```shell
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
```

Cuando Rust termine de instalarse, te pedirá que ejecutes un comando para instalar las herramientas de compilación de Rust en tu ruta. Debes ejecutar este comando o cerrar sesión y volver a iniciarla.

Una vez hecho esto, por favor, ejecute:

```shell
cd /opt/libreqos/src/
./build_rust.sh
```

La primera vez, esto llevará un tiempo, pero pondrá todo en el lugar correcto.

Ahora, para construir cajas de rust, corre:

```shell
cd rust
cargo build --all
```
