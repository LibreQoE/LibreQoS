# Integraciones

## Integración UISP

Primero, configure los parámetros relevantes para UISP (uispAuthToken, UISPbaseURL, etc.) en `/etc/lqos.conf`.

Para probar la integración de UISP, utilice

```shell
python3 integrationUISP.py
```

En la primera ejecución exitosa, se crearán los archivos network.json y ShapedDevices.csv.
Si existe un archivo network.json, no se sobrescribirá.
Puede modificar el archivo network.json para que refleje con mayor precisión los límites de ancho de banda.
ShapedDevices.csv se sobrescribirá cada vez que se ejecute la integración UISP.
Puede ejecutar IntegrationUISP.py automáticamente al arrancar y cada 10 minutos, lo cual es recomendable. Esto se puede habilitar configurando ```enable_uisp = true``` en `/etc/lqos.conf`

## Integración con Powercode 

Primero, configure los parámetros relevantes para Powercode (powercode_api_key, powercode_api_url, etc.) en`/etc/lqos.conf`.

Para probar la integración de Powercode, utilice

```shell
python3 integrationPowercode.py
```

En la primera ejecución exitosa, se creará un archivo ShapedDevices.csv.
Puede modificar manualmente el archivo network.json para reflejar los límites de ancho de banda del sitio/AP.
ShapedDevices.csv se sobrescribirá cada vez que se ejecute la integración de Powercode.
Tiene la opción de ejecutar IntegrationPowercode.py automáticamente al arrancar y cada 10 minutos, lo cual es recomendable. Esto se puede habilitar configurando ```enable_powercode = true``` en `/etc/lqos.conf`

## Integración con Sonar 

En primer lugar, configure los parámetros relevantes para Sonar (sonar_api_key, sonar_api_url, etc.) en `/etc/lqos.conf`.

Para probar la integración del sonar, utilice

```shell
python3 integrationSonar.py
```

En la primera ejecución exitosa, se creará un archivo ShapedDevices.csv.
Si existe un archivo network.json, no se sobrescribirá.
Puede modificar el archivo network.json para que refleje con mayor precisión los límites de ancho de banda.
ShapedDevices.csv se sobrescribirá cada vez que se ejecute la integración de Sonar.
Puede ejecutar IntegrationSonar.py automáticamente al arrancar y cada 10 minutos, lo cual es recomendable. Esto se puede habilitar configurando ```enable_sonar = true``` en `/etc/lqos.conf`

## Integración con Splynx 

Primero, configure los parámetros relevantes para Splynx (splynx_api_key, splynx_api_secret, etc.) en`/etc/lqos.conf`.

La integración de Splynx utiliza autenticación básica. Para usar este tipo de autenticación, asegúrese de habilitar [Acceso no seguro](https://splynx.docs.apiary.io/#introduction/authentication) en la configuración de su clave API de Splynx. Además, la clave API de Splynx debe tener acceso a los permisos necesarios.

Para probar la integración de Splynx, utilice

```shell
python3 integrationSplynx.py
```

En la primera ejecución exitosa, se creará un archivo ShapedDevices.csv.
Puedes crear manualmente el archivo network.json para reflejar con mayor precisión los límites de ancho de banda.
ShapedDevices.csv se sobrescribirá cada vez que se ejecute la integración de Splynx.
Tiene la opción de ejecutar IntegrationSplynx.py automáticamente al arrancar y cada 10 minutos, lo cual es recomendable. Esto se puede habilitar configurando ```enable_spylnx = true``` en `/etc/lqos.conf`.
