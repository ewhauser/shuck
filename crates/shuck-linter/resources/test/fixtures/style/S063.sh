#!/bin/sh
ln -s ../../share/doc/guide.pdf
ln -s ../share/doc/guide.pdf guide.pdf
ln -snf ../../etc/defaults cfg-link
ln ../../share/doc/guide.pdf guide-hardlink
ln -st /tmp ../../alpha ../../beta
ln -s "$base"/../../guide.pdf guide.pdf
command ln -s ../../wrapped/value wrapped
