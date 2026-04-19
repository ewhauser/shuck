#!/bin/sh
trap 'echo hi' ERR
trap 'echo debug' DEBUG
trap 'cleanup' RETURN
