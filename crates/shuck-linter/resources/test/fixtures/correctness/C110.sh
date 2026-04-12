#!/bin/bash
test "QT6=${QT6:-no}" = yes
[ yes = "QT6=${QT6:-no}" ]
[[ "QT6=${QT6:-no}" != yes ]]
[[ yes != "QT6=${QT6:-no}" ]]
