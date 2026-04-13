#!/bin/sh
echo one >> out.log
echo two >> out.log
echo three >> out.log

echo first >> semi.log; echo second >> semi.log; echo third >> semi.log

echo alpha >> "$log"
echo beta >> "$log"
echo gamma >> "$log"

echo no >> two.log
echo warn >> two.log

{ echo grouped one; echo grouped two; echo grouped three; } >> grouped.log
