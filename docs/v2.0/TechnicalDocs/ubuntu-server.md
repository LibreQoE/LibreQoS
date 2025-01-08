# Install Ubuntu Server

You can download Ubuntu Server 22.04 from <a href="https://releases.ubuntu.com/22.04/?_ga=2.149898549.2084151835.1707729318-1126754318.1683186906">here</a>.
At this time, only Ubuntu Server 22.04 is supported. Please be sure not to use other versions.

1. Boot Ubuntu Server from USB.
2. Follow the steps below to install Ubuntu Server.

<img width="1287" alt="01 select-language" src="https://github.com/user-attachments/assets/af33c525-129c-4ecc-9e35-0ca4fd69b192" />

<img width="1295" alt="02 keyboard" src="https://github.com/user-attachments/assets/08d3cd73-5144-414a-817b-d2a93ce40e01" />

<img width="1292" alt="03 version" src="https://github.com/user-attachments/assets/4917e389-5aa7-4636-a0f3-ba826b107d0b" />

For the network interfaces, disable the shaping interfaces entirely. Then, set a static IP address on the management interface (100.99.0.4 is just an example).
<img width="1293" alt="04 net int" src="https://github.com/user-attachments/assets/6d9b10a6-ea4e-45cf-a993-b21342c86772" />

<img width="1351" alt="05 no proxy" src="https://github.com/user-attachments/assets/f86ace56-d1b2-4cd0-88b4-5af1267153ea" />

<img width="1290" alt="06 download" src="https://github.com/user-attachments/assets/1a6b441d-548f-490c-89ae-3f1e9b8188ac" />

<img width="1286" alt="07 continue without updating" src="https://github.com/user-attachments/assets/29d385ad-928d-44f2-9fbe-4a14c72e4110" />

<img width="543" alt="08 use entire disk" src="https://github.com/user-attachments/assets/93c2cd00-229e-4206-9e51-a5c66b77ad5f" />

<img width="1288" alt="09 summary" src="https://github.com/user-attachments/assets/115297d5-5758-47b7-8ae4-6875027c68fd" />

<img width="1301" alt="10 user info" src="https://github.com/user-attachments/assets/03b521f2-cd8c-4178-bc0e-c6259a114059" />

<img width="1293" alt="11 skip ubuntu pro" src="https://github.com/user-attachments/assets/6e9d6bd1-45e2-4933-bf38-bfb454d019ac" />

Ensure SSH server is enabled so you can more easily log into the server later.
<img width="1291" alt="12 openssh" src="https://github.com/user-attachments/assets/983fa8c5-037c-435a-9e39-01d177615001" />

<img width="1291" alt="13 skip these" src="https://github.com/user-attachments/assets/e263bb76-678f-4bcf-b382-942fc48279ab" />

<img width="1290" alt="14 reboot" src="https://github.com/user-attachments/assets/5dd24b80-3586-43b9-ac27-75fd2095728e" />

You can use scp or sftp to access files from your LibreQoS server for easier file editing. Here's how to access via scp or sftp using an [Ubuntu](https://www.addictivetips.com/ubuntu-linux-tips/sftp-server-ubuntu/) or [Windows](https://winscp.net/eng/index.php) machine.
