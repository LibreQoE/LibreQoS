# Comparte tu antes y después

Le pedimos que comparta una captura de pantalla anónima de su implementación de LibreQoS antes (modo de solo monitor) y después (cola habilitada) en el [Chat LibreQoS ](https://chat.libreqos.io/join/fvu3cerayyaumo377xwvpev6/). T
Esto nos ayuda a evaluar el impacto de nuestro software. Y también nos alegra.

1. Habilitar el modo de solo monitor
2. Modo Klingon (Redactar información del cliente)
3. Captura de pantalla
4. Reanudar las colas regulares
5. Captura de pantalla

## Habilitar el modo de solo monitor

```shell
sudo systemctl stop lqos_scheduler
sudo systemctl restart lqosd
sudo systemctl restart lqos_node_manager
```

## Modo Klingon 

Vaya a la interfaz web y haga clic en Configuración. Active la opción "Redactar información del cliente" (modo de captura de pantalla) y luego "Aplicar cambios".

## Reanudar la cola normal

```shell
sudo systemctl start lqos_scheduler
```

## Captura de pantalla

Para generar una captura de pantalla, acceda a la interfaz web y haga clic en Configuración. Active "Ocultar información del cliente" (modo de captura de pantalla), "Aplicar cambios" y, a continuación, vuelva al panel de control para realizar una captura de pantalla.
