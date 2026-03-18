# Insumos para Desarrollo Futuro

Esta pagina resume insumos recurrentes de feedback operativo revisados para LibreQoS, en el contexto de entregables soportados por NLNet.

## Alcance y Metodo

Ventana de revision:

- 1 de marzo de 2024 a 1 de marzo de 2026 (UTC)

Fuentes de entrada:

- Issues de GitHub del repositorio LibreQoS
- Canales comunitarios de soporte

Volumen revisado:

- 139 issues de GitHub
- 10,275 mensajes de canales comunitarios

## Temas Recurrentes

Estos temas ayudan a priorizar trabajo futuro. No son compromisos de release.

## 1) Claridad de UI y confianza operativa

Sintomas ejemplo:

- Vistas en blanco o parcialmente vacias
- Falta de contexto para diagnostico
- Senales de estado inconsistentes entre pantallas

Issues representativos:

- [#922 - Flowmap doesn't render](https://github.com/LibreQoE/LibreQoS/issues/922)
- [#921 - ASN Explorer dropdowns are empty](https://github.com/LibreQoE/LibreQoS/issues/921)
- [#920 - Tree Overview shows blank on low-traffic boxes](https://github.com/LibreQoE/LibreQoS/issues/920)
- [#831 - Dashboard goes blank when used by same user in multiple locations](https://github.com/LibreQoE/LibreQoS/issues/831)

## 2) Comportamiento de integraciones y seguridad de fuente de verdad

Sintomas ejemplo:

- Confusion sobre ownership de overwrite de archivos de shaping
- Casos borde de matching en integraciones
- Defaults poco claros durante onboarding

Issues representativos:

- [#860 - always_overwrite_network_json default behavior confusion](https://github.com/LibreQoE/LibreQoS/issues/860)
- [#899 - UISP: always_overwrite_network_json=false and missing ShapedDevices.csv](https://github.com/LibreQoE/LibreQoS/issues/899)
- [#845 - UISP: multi-services with same site name](https://github.com/LibreQoE/LibreQoS/issues/845)
- [#699 - UISP: trailing spaces break matching](https://github.com/LibreQoE/LibreQoS/issues/699)

## 3) Friccion en onboarding y despliegue temprano

Sintomas ejemplo:

- Quiebres de primer arranque
- Confusion en flujo de startup/configuracion
- Mismatch entre defaults de instalador y expectativas

Issues representativos:

- [#859 - Broken default ShapedDevices.csv from setup tool](https://github.com/LibreQoE/LibreQoS/issues/859)
- [#858 - Config tool webusers flow break](https://github.com/LibreQoE/LibreQoS/issues/858)
- [#728 - Default installation bridge-mode mismatch](https://github.com/LibreQoE/LibreQoS/issues/728)
- [#667 - Add YAML creation to setup installer](https://github.com/LibreQoE/LibreQoS/issues/667)

## 4) Guardrails de escala/topologia y advertencias proactivas

Sintomas ejemplo:

- Presion de profundidad de colas en jerarquias complejas
- Necesidad de mejor visibilidad de riesgo de overflow
- Higiene de parent nodes y claridad topologica

Issues representativos:

- [#913 - Tree verbosity and HTB depth pressure](https://github.com/LibreQoE/LibreQoS/issues/913)
- [#801 - Visible warning for TC ID overflow](https://github.com/LibreQoE/LibreQoS/issues/801)
- [#856 - Improve no-parent circuit warnings](https://github.com/LibreQoE/LibreQoS/issues/856)
- [#560 - htb too many events under load](https://github.com/LibreQoE/LibreQoS/issues/560)

## 5) Ajuste de rendimiento y encaje de hardware

Sintomas ejemplo:

- Comportamiento de utilizacion de cores en CPUs heterogeneas
- Preocupaciones de crecimiento de memoria
- Ajuste de throughput/headroom por clase de hardware

Issues representativos:

- [#928 - Detect e-cores and avoid shaping load there](https://github.com/LibreQoE/LibreQoS/issues/928)
- [#651 - Memory growth even when gather_stats is false](https://github.com/LibreQoE/LibreQoS/issues/651)
- [#578 - Resource footprint for 100 Gbit fiber links](https://github.com/LibreQoE/LibreQoS/issues/578)
- [#526 - Performance improvements during reloading](https://github.com/LibreQoE/LibreQoS/issues/526)

## Direcciones Candidatas Bajo Evaluacion

1. Mejorar diagnostico guiado y claridad de empty states en WebUI.
2. Hacer mas explicito ownership de fuente de verdad y overwrite.
3. Expandir validaciones pre-flight y manejo de edge cases en integraciones.
4. Reforzar guardrails de escala y ergonomia de advertencias tempranas.
5. Expandir runbooks para arquitecturas comunes.
6. Mejorar guia de encaje rendimiento/topologia/hardware.

## Fuera de Alcance

- Esta pagina es un resumen de insumos de planificacion, no un roadmap comprometido.
- Incluir un issue aqui no garantiza objetivo de release ni fecha de implementacion.

## Paginas Relacionadas

- [Recetas de Despliegue](recipes-es.md)
- [Casos de Estudio (Anonimizados)](case-studies-es.md)
- [Troubleshooting](troubleshooting-es.md)
