#!/usr/bin/env bash

top_level=1

outer() {
  local local_name=2
  echo "$top_level"
  echo "${top_level}"
  echo ok; local to
}

other() {
  local hidden_name=3
  :
}

build() {
  :
}

b
