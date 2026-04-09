#!/bin/sh
echo ok 2&>1
echo ok 2&>>1
echo ok &>1
echo ok &>>1
echo ok 2 &>1
echo ok &>01

echo ok 2>&1
echo ok >&1
echo ok &>file
echo ok 2&>file
echo ok &>"1"
echo ok &>-1
echo ok &>+1
