#!/bin/sh
a=1
b=jpg
if [ -n "$a" ] && ( [ "$b" = jpeg ] || [ "$b" = jpg ] ); then echo ok; fi
if ! ( [ "$b" = jpeg ] || [ "$b" = jpg ] ); then echo ok; fi
( { [ "$b" = jpeg ] || [ "$b" = jpg ]; } )
( [ "$b" = jpeg ] ; [ "$b" = jpg ] )
( [ "$b" = jpeg ] )
