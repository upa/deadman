deadman
=======

deadmanはCLI上で動作する複数のホストの生存確認をするためのソフトウェア
です。よしなにライブラリをインストールしてご利用ください。

メールで通知を飛ばすといった贅沢な機能はございません。ひたすら監視する
だけです。イベント等でのネットワーク敷設など、短期決戦の場でのご利用が
おすすめです。


How to use
==========

まずconfigを書きます。configの中身は下記のような感じ。

	 % cat deadman.conf
	 Google_DNS	=	8.8.8.8
	 kame6		=	2001:200:dff:fff1:216:3eff:feb1:44d7


1行ごとに、空白を含まない文字列(画面に表示されるときの名前)に対して監視
するIPv4またはIPv6アドレスを指定します。あとはconfigを指定して起動するだけ
です。


 sudo ./deadman -c deadman.conf

Net::PingがICMPを吐くためにroot privilegeが必要なのでsudoしてください。
これだけです。そうするとこんな感じになります。


	                                     Dead Man
	    local.host (x.x.x.x)                                    [ver 2.010.12]
	    Keytype: [a] Display addr, [i] ping interval, [t] ping timeout, [q]uit
	 
	    HOSTNAME    ADDRESS                  LOSS%   RTT  AVG  SNT  RESULT
	    Google_DNS  8.8.8.8           UP        0%    34   35    3  ...
	  > kame6       2001:200:dff:ff   UP        0%     1    2    3  ..
 



Copyright
=========
INTEROP TOKYO SHOWNET TEAM all rights reserved.

Contact
=======
upa@haeena.net

