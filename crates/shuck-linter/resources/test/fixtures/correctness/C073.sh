 #!/bin/sh
:

# should not trigger: only the file header counts as the shebang
#!/bin/sh

# should not trigger: spacing after `#!` is a different rule
#! /bin/sh
