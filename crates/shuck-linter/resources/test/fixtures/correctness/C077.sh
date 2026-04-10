#!/bin/bash
start=40
tool=hello
spaces=$(($start - $(echo "$tool" | wc -c)))
echo "$spaces"
