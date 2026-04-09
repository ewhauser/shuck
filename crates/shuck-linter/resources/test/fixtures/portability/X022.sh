#!/bin/sh
wait -n
wait -pn x
wait -f -n %1
wait -x
wait -1
