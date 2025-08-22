# LibreQoS Long-Term Stats (LTS)

## Acerca de LTS
Obtén más información sobre LTS en nuestro sitio web, [aquí](https://libreqos.io/lts/).

## Registro
Encuentre su ID de Nodo LibreQoS iniciando sesión en su equipo donde se encuentre LibreQoS por SSH y ejecutando:
```
sed -n 's/node_id = //p' /etc/lqos.conf | sed -e 's/^"//' -e 's/"$//'`
```
El resultado será un número largo: el identificador único de su caja LibreQoS.

Ahora visite:
```
https://stats.libreqos.io/trial1/YOUR_NODE_ID
```
Donde YOUR_NODE_ID es el ID de Nodo que encontró en el paso anterior.

Si es la primera vez que usa LTS, seleccione "Sign Up - Regular Long-Term Stats".

Complete el registro para la prueba gratuita de 30 días ingresando su información de pago.

El proceso de registro le proporcionará una Clave de Licencia LTS.

Regrese a la caja de LibreQoS y edite el archivo `/etc/lqos.conf` para modificar la sección [long_term_stats] de la siguiente manera:
```
[long_term_stats]
gather_stats = true
collation_period_seconds = 60
license_key = "YOUR_LICENSE_KEY"
uisp_reporting_interval_seconds = 300
```
Donde YOUR_LICENSE_KEY es su clave única de licencia LTS obtenida en el paso anterior. Asegúrese de incluir las comillas.

Ahora guarde el archivo y ejecute: `sudo systemctl restart lqosd lqos_scheduler`. Esto reiniciará el proceso lqosd, permitiéndole comenzar a enviar datos a LTS.

## Acceso a LTS
Para acceder a LTS, visite [https://stats.libreqos.io/](https://stats.libreqos.io/) - e ingrese su clave LTS, usuario (correo electrónico) y contraseña.
