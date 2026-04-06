#!/bin/bash

arr=(alpha "two words")
printf '%s\n' ${arr[@]}
printf '%s\n' ${arr[*]}

printf '%s\n' "${arr[@]}"
printf '%s\n' "${arr[*]}"
printf '%s\n' ${arr[0]}
