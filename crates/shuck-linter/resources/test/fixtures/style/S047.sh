#!/bin/bash
LAYOUTS="$(ls layout.*.h | cut -d. -f2 | xargs echo)"
