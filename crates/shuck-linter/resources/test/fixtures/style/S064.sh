#!/bin/sh
find . -type d -name CVS | xargs -iX rm -rf X
find . -type d -name CVS | xargs -0iX rm -rf X
find . -type d -name CVS | xargs -i{} rm -rf '{}'
command xargs -i echo {}
sudo xargs -i echo {}
find . -type d -name CVS | xargs -I{} rm -rf {}
find . -type d -name CVS | xargs --replace rm -rf {}
find . -type d -name CVS | xargs --replace={} rm -rf '{}'
find . -type d -name CVS | xargs -0 rm -rf
find . -type d -name CVS | xargs --null rm -rf
