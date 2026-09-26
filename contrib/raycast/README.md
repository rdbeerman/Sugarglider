# Raycast script commands

`switch-context.sh` is a [Raycast script command](https://github.com/raycast/script-commands)
that runs `sugarglider context switch` with the text you type.

## Add the folder to Raycast

Open Raycast, go to Extensions, select the Script Commands tab, and click "Add
Script Directory". Choose this `contrib/raycast` folder. Raycast then lists
"Switch Context" among your commands. Type part of a context name or its
number and press Enter.

The script runs `/usr/local/bin/sugarglider`, where `make install` puts the
binary. Turn contexts on with `settings.experimental.contexts.enable = true`
in the config file, and keep Sugarglider running.

## SuperCmd

SuperCmd's documentation says it imports Raycast script-command folders. Nobody
has tested that with Sugarglider, so the script may need changes there.
