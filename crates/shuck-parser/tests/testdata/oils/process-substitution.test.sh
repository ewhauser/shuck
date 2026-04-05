## compare_shells: bash

#### cat from process substitution
cat <(printf '%s\n' alpha beta)

#### while read from process substitution
while IFS= read -r line; do
  printf 'loop<%s>\n' "$line"
done < <(printf '%s\n' 'sp ace' 'ba\ck')

#### mapfile from process substitution
mapfile -t arr < <(printf '%s\n' one two)
argv.sh "${arr[@]}"

#### write to process substitution
printf '%s\n' alpha beta > >(sed 's/^/ps:/')
