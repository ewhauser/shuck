#!/bin/sh
tr '[:upper:]' [:lower:]
tr [:upper:] [:lower:]
tr [:alpha:] x
tr x [:alpha:]
command tr [:upper:] [:lower:]
tr '[[:upper:]]' [:lower:]
tr '[:upper:]' '[:lower:]'
