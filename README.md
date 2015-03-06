deadman
=======

deadman is, a ovservation software for host status by ping.

deadman does not have rich functionalities. It only check the host 
status by ICMP. We recomend using deadman for building temporary network
such as conference or event networks.


![demo](https://github.com/upa/deadman/raw/pydman/img/deadman-demo.gif)

how to use
==========

at first, write a config file like below.

	% cat deadman.conf
	google          173.194.117.176
	googleDNS       8.8.8.8
	kame            203.178.141.194
	kame6           2001:200:dff:fff1:216:3eff:feb1:44d7

one line on config file indicate a observed host.
IPv6 address is naturally supported.

then,

	% ./deadman deadman.conf


Moreover, -s option indicates the scale of RTT bar graph. default is 10ms.


Copyright
=========

2015 Interop Tokyo ShowNet team All Rights Reserved.


Contact
=======

upa@haeena.net
