# Receta: Event WiFi con Shaping por Grupos de Subred

Use este patron para redes de eventos de alta densidad y corta duracion, donde agrupar clientes por subred es mas operativo que administrar ciclo de vida por dispositivo.

## Ajuste

- Mejor para: eventos temporales y alta rotacion de clientes.
- Evitar cuando: necesita enforcement estricto por dispositivo a largo plazo.

## Patron

- Defina un circuito por grupo de subred (por ejemplo por `/24`).
- Mantenga topologia simple (`flat` o jerarquia poco profunda).
- Use `Parent Node` explicito en `ShapedDevices.csv`.

Ejemplo de fila:

```text
Circuit ID,Circuit Name,Device ID,Device Name,Parent Node,MAC,IPv4,IPv6,Download Min Mbps,Upload Min Mbps,Download Max Mbps,Upload Max Mbps,Comment
EVT-24-101,Event Hall A Subnet,EVT-24-101,HallA-Clients,Event_Core,,100.64.101.0/24,,10,10,300,300,Temporary event subnet group
```

Si el modo de integracion controla sus archivos de shaping, estas ediciones directas pueden sobrescribirse en el siguiente sync.

## Implementacion

1. Confirme un solo owner para datos de shaping.
2. Prepare circuitos por subred para segmentos esperados.
3. Mantenga jerarquia shallow para reducir churn de colas.
4. Valide en WebUI antes de abrir trafico de registro.

## Checklist de Validacion

1. El trafico aparece bajo los circuitos esperados.
2. Scheduler se mantiene saludable bajo churn de clientes.
3. Presion de queue/class estable (incluyendo overflow warnings).
4. Vistas Flow/Tree coherentes en picos.

## Rollback

1. Restaure `ShapedDevices.csv` y `network.json` pre-evento.
2. Reinicie scheduler.
3. Valide comportamiento base.

## Paginas Relacionadas

- [Referencia de Configuracion Avanzada](configuration-advanced-es.md)
- [Planeacion de Escala y Topologia](scale-topology-es.md)
- [Troubleshooting](troubleshooting-es.md)
