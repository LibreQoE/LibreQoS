/ip address add address=198.18.10.1/24 interface=ether2
/ip address add address=198.18.30.1/24 interface=ether4
/ip pool add name=radius-pppoe ranges=100.64.0.2-100.64.0.2
/ppp profile add name=radius-pppoe local-address=100.64.0.1 remote-address=radius-pppoe use-encryption=no
/interface pppoe-server server add interface=ether3 service-name=radius-lab default-profile=radius-pppoe authentication=pap one-session-per-host=yes disabled=no
/radius add service=ppp address=198.18.10.10 authentication-port=1812 accounting-port=1813 secret=__RADIUS_SHARED_SECRET__
/ppp aaa set use-radius=yes accounting=yes interim-update=5s
/ip service set ssh disabled=no
