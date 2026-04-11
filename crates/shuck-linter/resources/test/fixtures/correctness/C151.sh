#!/bin/bash

# Invalid: commas inside unquoted array values become part of one element.
parts=(alpha,beta)
parts=(alpha, beta)
parts+=(gamma,delta)
declare -a flags=(--one,--two)
declare -A assoc=([left]=1, [right]=2)
values=(head,$tail)
mixed=(head,{left,right},tail)

# Valid: whitespace-separated array elements.
parts_ok=(alpha beta)
parts_ok+=(gamma delta)
declare -A assoc_ok=([left]=1 [right]=2)

# Valid: commas in quoted words are literal data.
quoted1=("alpha,beta")
quoted2=('gamma,delta')

# Valid: brace expansion comma lists are not array separators.
brace=({one,two})
brace_paths=({$XDG_CONFIG_HOME,$HOME}/{alacritty,}/{.,}alacritty.ym?)
