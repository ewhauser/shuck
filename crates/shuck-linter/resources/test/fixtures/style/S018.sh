#!/bin/bash

arr=($(printf '%s\n' a b))
arr=(`printf '%s\n' c d`)
arr=(prefix$(printf '%s\n' e f)suffix)
declare listed=($(printf '%s\n' one two))
arr+=($(printf '%s\n' tail))

arr=("$(printf '%s\n' safe)")
arr=([0]=$(printf '%s\n' keyed))
value=$(printf '%s\n' scalar)
