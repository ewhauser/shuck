#!/bin/bash
true
&& echo x

true
|| echo y

echo hi
| cat

true &&
  echo ok

echo hi |
  cat
