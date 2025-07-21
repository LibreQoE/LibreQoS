# Instalar Servidor de Ubuntu

Puedes descargar Ubuntu Server 24.04 LTS desde <a href="https://ubuntu.com/download/server">aquí</a>.
Por el momento, solo se admite Ubuntu Server 24.04. Asegúrate de no utilizar otras versiones.

1. Arranca Ubuntu Server desde una USB.

2. Sigue los pasos a continuación para instalar Ubuntu Server.

<img width="1287" alt="01 select-language" src="https://github.com/user-attachments/assets/af33c525-129c-4ecc-9e35-0ca4fd69b192" /> 

<img width="1295" alt="02 keyboard" src="https://github.com/user-attachments/assets/08d3cd73-5144-414a-817b-d2a93ce40e01" /> 

<img width="1292" alt="03 version" src="https://github.com/user-attachments/assets/4917e389-5aa7-4636-a0f3-ba826b107d0b" />

Para las interfaces de red, desactiva completamente las interfaces utilizadas. Después, asigna una dirección IP estática a la interfaz de administración (100.99.0.4 es solo un ejemplo).
<img width="1293" alt="04 net int" src="https://github.com/user-attachments/assets/6d9b10a6-ea4e-45cf-a993-b21342c86772" />

<img width="1351" alt="05 no proxy" src="https://github.com/user-attachments/assets/f86ace56-d1b2-4cd0-88b4-5af1267153ea" /> 

<img width="1290" alt="06 download" src="https://github.com/user-attachments/assets/1a6b441d-548f-490c-89ae-3f1e9b8188ac" /> 

<img width="1286" alt="07 continue without updating" src="https://github.com/user-attachments/assets/29d385ad-928d-44f2-9fbe-4a14c72e4110" /> 

<img width="543" alt="08 use entire disk" src="https://github.com/user-attachments/assets/93c2cd00-229e-4206-9e51-a5c66b77ad5f" /> 

<img width="1288" alt="09 summary" src="https://github.com/user-attachments/assets/115297d5-5758-47b7-8ae4-6875027c68fd" /> 

<img width="1301" alt="10 user info" src="https://github.com/user-attachments/assets/03b521f2-cd8c-4178-bc0e-c6259a114059" /> 

<img width="1293" alt="11 skip ubuntu pro" src="https://github.com/user-attachments/assets/6e9d6bd1-45e2-4933-bf38-bfb454d019ac" />

Asegúrese de que el servidor SSH esté habilitado para que pueda iniciar sesión en el servidor más fácilmente después.
<img width="1291" alt="12 openssh" src="https://github.com/user-attachments/assets/983fa8c5-037c-435a-9e39-01d177615001" />

<img width="1291" alt="13 skip these" src="https://github.com/user-attachments/assets/e263bb76-678f-4bcf-b382-942fc48279ab" /> 

<img width="1290" alt="14 reboot" src="https://github.com/user-attachments/assets/5dd24b80-3586-43b9-ac27-75fd2095728e" />

Puedes utilizar scp o sftp para acceder a los archivos de tu servidor LibreQoS y facilitar la modifición de archivos. Aquí se explica cómo acceder mediante scp o sftp usando una máquina [Ubuntu](https://www.addictivetips.com/ubuntu-linux-tips/sftp-server-ubuntu/) o [Windows](https://winscp.net/eng/index.php).

