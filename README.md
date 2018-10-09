deadman
=======

deadman is, an observation software for host status using ping.

deadman does not have rich functionalities. It only checks the host
status using ICMP echo. We recomend using deadman for building
temporary networks such as conference and event networks. This
software was originally designed and implemented for Interop Tokyo
ShowNet.


![demo](https://github.com/upa/deadman/raw/master/img/deadman-demo.gif)

how to use
==========

At first, write a config file like below.

	% cat deadman.conf
	google          173.194.117.176
	googleDNS       8.8.8.8
	kame            203.178.141.194
	kame6           2001:200:dff:fff1:216:3eff:feb1:44d7

A line on the config file indicates an observed host.
IPv6 address is naturally supported.

then,

	% ./deadman deadman.conf


Moreover, -s option indicates the scale of RTT bar graph. default is 10ms.

ping via remote host through ssh is implemented.
For example, 

	google-via-ssh  173.194.117.176 relay=X.X.X.X os=Linux

this line means sending ping to a google server via remote server X.X.X.X.
username and ssh-key for remote host can be specified by _user=USER_,
_key=KEYPATH_. Other ssh attributes follow user's environment executing
deadman.

You can send deadman a SIGHUP to have it reload its configuration file.
When this happens, existing entries will not lose their history.


License
=======

MIT


Contact
=======

upa@haeena.net
