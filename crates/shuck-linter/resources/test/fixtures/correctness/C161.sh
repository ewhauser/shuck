#!/bin/bash

early_call

defined_first() {
  echo ok
}
defined_first

wrapper() {
  nested_later
}

driver() {
  wrapper
}
driver

nested_later() {
  echo nested
}

early_call() {
  echo late
}
