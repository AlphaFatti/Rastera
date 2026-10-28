-- Operate only the normal System Settings checkbox; never write preference files.
-- Usage: osascript tools/reduce_motion.applescript read|on|off
on run argv
    set requested to item 1 of argv
    if requested is not in {"read", "on", "off"} then error "Expected read, on or off"
    tell application "System Events"
        tell process "System Settings"
            set pane to group 1 of group 2 of splitter group 1 of group 1 of window "Display"
            set paneItems to entire contents of pane
            repeat with itemRef in paneItems
                if class of itemRef is checkbox then
                    if name of itemRef is "Reduce motion" then
                        set originalValue to value of itemRef
                        if requested is not "read" then
                            set desiredValue to 0
                            if requested is "on" then set desiredValue to 1
                            if originalValue is not desiredValue then click itemRef
                            repeat 30 times
                                if value of itemRef is desiredValue then exit repeat
                                delay 0.1
                            end repeat
                            if value of itemRef is not desiredValue then error "Reduce motion checkbox did not change"
                        end if
                        return "original=" & originalValue & " readback=" & (value of itemRef)
                    end if
                end if
            end repeat
            error "Reduce motion checkbox not found in Accessibility > Display"
        end tell
    end tell
end run
