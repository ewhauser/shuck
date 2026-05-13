#!/bin/sh

0=demo
f() { 0=demo; }
0=demo env
+0=demo

export 0=demo
command 0=demo
0+=demo
00=demo
"0=demo"
