# Install Ubuntu Server

**Version: LibreQoS v3.**

LibreQoS v3 needs Ubuntu Server 24.04 LTS or newer. Download it from
[ubuntu.com](https://ubuntu.com/download/server), boot it from USB, and follow
the installer. The screenshots below are from 24.04.

<img width="1287" alt="01 select-language" src="https://github.com/user-attachments/assets/af33c525-129c-4ecc-9e35-0ca4fd69b192" />

<img width="1295" alt="02 keyboard" src="https://github.com/user-attachments/assets/08d3cd73-5144-414a-817b-d2a93ce40e01" />

<img width="1292" alt="03 version" src="https://github.com/user-attachments/assets/4917e389-5aa7-4636-a0f3-ba826b107d0b" />

**Network:** disable the dataplane (shaping) interfaces entirely; LibreQoS hands
them to DPDK later, and they must have no address. Give the management
interface a static IP address (192.0.2.4 below is only an example).

<img width="1293" alt="04 net int" src="https://github.com/user-attachments/assets/6d9b10a6-ea4e-45cf-a993-b21342c86772" />

<img width="1351" alt="05 no proxy" src="https://github.com/user-attachments/assets/f86ace56-d1b2-4cd0-88b4-5af1267153ea" />

<img width="1290" alt="06 download" src="https://github.com/user-attachments/assets/1a6b441d-548f-490c-89ae-3f1e9b8188ac" />

<img width="1286" alt="07 continue without updating" src="https://github.com/user-attachments/assets/29d385ad-928d-44f2-9fbe-4a14c72e4110" />

<img width="543" alt="08 use entire disk" src="https://github.com/user-attachments/assets/93c2cd00-229e-4206-9e51-a5c66b77ad5f" />

<img width="1288" alt="09 summary" src="https://github.com/user-attachments/assets/115297d5-5758-47b7-8ae4-6875027c68fd" />

<img width="1301" alt="10 user info" src="https://github.com/user-attachments/assets/03b521f2-cd8c-4178-bc0e-c6259a114059" />

<img width="1293" alt="11 skip ubuntu pro" src="https://github.com/user-attachments/assets/6e9d6bd1-45e2-4933-bf38-bfb454d019ac" />

**SSH:** enable the OpenSSH server so you can log in remotely.

<img width="1291" alt="12 openssh" src="https://github.com/user-attachments/assets/983fa8c5-037c-435a-9e39-01d177615001" />

<img width="1291" alt="13 skip these" src="https://github.com/user-attachments/assets/e263bb76-678f-4bcf-b382-942fc48279ab" />

<img width="1290" alt="14 reboot" src="https://github.com/user-attachments/assets/5dd24b80-3586-43b9-ac27-75fd2095728e" />

After the reboot, continue with [Installing LibreQoS v3](install.md). To edit
files on the host from another machine, use scp or sftp, for example
[WinSCP](https://winscp.net/eng/index.php) on Windows.
