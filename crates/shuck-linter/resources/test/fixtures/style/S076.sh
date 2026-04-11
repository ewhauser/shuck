#!/bin/bash
printf '%s\n' "left "middle" right" "foo"-"bar"
name="foo"bar"baz"
declare local_name="foo"bar"baz"
if [ "foo"bar"baz" = x ]; then :; fi
case "foo"bar"baz" in x) : ;; esac
case x in "foo"bar"baz") : ;; esac
printf '%s\n' 'foo'bar'baz'
printf '%s\n' "foo"${bar}"baz" "foo"$(printf '%s' x)"baz"
printf '%s\n' "foo"/"bar" "foo"="bar" "foo":"bar" "foo"?"bar"
if [[ x =~ "foo"bar"baz" ]]; then :; fi
