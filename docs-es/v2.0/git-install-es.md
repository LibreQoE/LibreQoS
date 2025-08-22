# Instalación vía Git (Para desarrolladores – No recomendado)

## Clonar el repositorio

La ubicación recomendada para la instalación es `/opt/libreqos`
Diríjase a dicha ubicación e inicie la clonación del repositorio:

```shell
cd /opt/
sudo git clone https://github.com/LibreQoE/LibreQoS.git libreqos
sudo chown -R $USER /opt/libreqos
cd /opt/libreqos/
git pull
```

Al especificar `libreqos` al final, Git asegurará que el nombre del directorio esté en minúsculas.

## Instalar dependencias con apt y pip

Es necesario instalar ciertos paquetes mediante `apt`:

```shell
sudo apt-get install -y python3-pip clang mold esbuild gcc gcc-multilib llvm libelf-dev git nano graphviz curl screen llvm pkg-config linux-tools-common linux-tools-`uname -r` libbpf-dev libssl-dev
```

Posteriormente, debe instalar algunas dependencias de Python:

```shell
cd /opt/libreqos
PIP_BREAK_SYSTEM_PACKAGES=1 pip install -r requirements.txt
sudo PIP_BREAK_SYSTEM_PACKAGES=1 pip install -r requirements.txt
```

## Instalar el entorno de desarrollo de Rust

Ejecute el siguiente comando:

```shell
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
```

Una vez finalizada la instalación de Rust, se le indicará que ejecute un comando para añadir las herramientas de construcción de Rust a su variable de entorno PATH. Deberá ejecutar dicho comando o bien cerrar sesión y volver a iniciarla.

Después, ejecute lo siguiente:

```shell
cd /opt/libreqos/src/
./build_rust.sh
```

Este proceso tomará algo de tiempo la primera vez, pero colocará todos los componentes en las ubicaciones correspondientes.

Ahora, para compilar los crates de Rust, ejecute:

```shell
cd rust
cargo build --all
```

## Lqos.conf

Copie el archivo de configuración lqos.conf al directorio `/etc`. Este paso no es necesario si realizó la instalación utilizando el archivo .deb:

```shell
cd /opt/libreqos/src
sudo cp lqos.example /etc/lqos.conf
```

## Configuración

Proceda a configurar los parámetros [siguiendo esta guía](configuration-es.md).

## Configuración de servicios (Daemons)

## Ejecutar los servicios con systemd

Nota: Si utilizó el instalador .deb, puede omitir esta sección. Dicho instalador configura los servicios automáticamente.

Ahora puede configurar los servicios `lqosd` y `lqos_scheduler` para que se ejecuten como servicios gestionados por systemd.

```shell
sudo cp /opt/libreqos/src/bin/lqosd.service.example /etc/systemd/system/lqosd.service
sudo cp /opt/libreqos/src/bin/lqos_scheduler.service.example /etc/systemd/system/lqos_scheduler.service
```

Finalmente, ejecute:

```shell
sudo systemctl daemon-reload
sudo systemctl enable lqosd lqos_scheduler
sudo systemctl start lqosd lqos_scheduler
```

Ahora puede abrir un navegador web en `http://a.b.c.d:9123` (reemplace `a.b.c.d` con la dirección IP de administración de su servidor de shaping) y disfrutar de una vista en tiempo real de su red.
