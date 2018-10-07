#!/bin/bash
# Travis runs Ubuntu 14.04 and has a really old version of zmq that doesn't work for us, install a newer one.
echo "deb https://anorien.csc.warwick.ac.uk/mirrors/download.opensuse.org/repositories/network:/messaging:/zeromq:/release-stable/xUbuntu_14.04/ ./" \
    >>/etc/apt/sources.list
wget https://anorien.csc.warwick.ac.uk/mirrors/download.opensuse.org/repositories/network:/messaging:/zeromq:/release-stable/xUbuntu_14.04/Release.key -O- \
    | sudo apt-key add
apt-get update
apt-get install -y libzmq3-dev
