deadman
=======

deadman is an observation software for host status using ping.

deadman does not have rich functionalities. It only checks host
statuses using ICMP echo. We recomend using deadman for building
temporary networks such as conference and event networks. This
software was originally designed and implemented for Interop Tokyo
ShowNet.


![demo](https://github.com/upa/deadman/raw/master/img/deadman-demo.gif)

How to use
==========

Clone this repository and then run.

	$ git clone https://github.com/upa/deadman
	$ cd deadman
	$ ./deadman deadman.conf


To change the targets, modify or create a config file.

	$ cat deadman.conf
	google          173.194.117.176
	googleDNS       8.8.8.8
	kame            203.178.141.194
	kame6           2001:200:dff:fff1:216:3eff:feb1:44d7

Each line indicates a target host. Ping options, specifying source
addresses and using netns, etc, are noted on the deadman.conf. For
example, ping via a remote host through ssh is implemented.

	google-via-ssh  173.194.117.176 relay=X.X.X.X os=Linux

This line means sending ping to a google server via the remote server
X.X.X.X. username and ssh-key for the remote host can be specified by
_user=USER_, _key=KEYPATH_. Other ssh attributes follow user's
environment executing deadman.

Moreover, -s option indicates the scale of RTT bar graph. default is 10ms.

You can send deadman a SIGHUP to have it reload its configuration file.
When this happens, existing entries will not lose their history.


License
=======

MIT


Contact
=======

upa@haeena.net
