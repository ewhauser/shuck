#!/bin/sh

# Invalid: bare C-style close marker parsed as a command.
*/

# Valid: glob marker used as an argument.
echo */

# Valid: slash-star sequence in quoted text.
printf '%s\n' "*/"
