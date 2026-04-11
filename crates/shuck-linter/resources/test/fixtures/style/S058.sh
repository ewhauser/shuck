#!/bin/sh

mkdir $dir
mkdir -p $PKG/var/lib/app
mkdir -m 750 prefix$leaf
mkdir --mode=700 ${root}/bin
command mkdir $other

mkdir "$dir"
mkdir -- "$dir"
mkdir -m $mode "$dir"
mkdir --mode=$mode "$dir"
mkdir --mode "$mode" "$dir"
mkdir -pm 750 "$dir"
