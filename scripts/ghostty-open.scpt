on labelManagedTerminal(targetTerm, managedTitle)
	tell application "Ghostty"
		try
			perform action "set_surface_title:" & managedTitle on targetTerm
		end try
	end tell
end labelManagedTerminal

on sendAttachCommand(targetTerm, attachCommand)
	tell application "Ghostty"
		input text attachCommand to targetTerm
		send key "enter" to targetTerm
	end tell
end sendAttachCommand

on resizePreview(anchorTerm, targetWindow)
	try
		tell application "Ghostty"
			set windowBounds to bounds of targetWindow
			set windowWidth to item 3 of windowBounds - item 1 of windowBounds
			set resizeDelta to (windowWidth / 6) as integer
			if resizeDelta > 0 then
				perform action "resize_split:right," & resizeDelta on anchorTerm
			end if
		end tell
	end try
end resizePreview

on terminalById(targetTab, targetId)
	if targetId is "" then return missing value

	tell application "Ghostty"
		repeat with candidateTerm in terminals of targetTab
			try
				if (id of candidateTerm as text) is targetId then return candidateTerm
			end try
		end repeat
	end tell

	return missing value
end terminalById

on firstManagedTerminal(targetTab, managedTitlePrefix)
	tell application "Ghostty"
		repeat with candidateTerm in terminals of targetTab
			try
				set termTitle to name of candidateTerm
				if termTitle starts with managedTitlePrefix then return candidateTerm
			end try
		end repeat
	end tell

	return missing value
end firstManagedTerminal

on anchorTerminalFor(targetTab, managedTerm)
	tell application "Ghostty"
		repeat with candidateTerm in terminals of targetTab
			if managedTerm is missing value then return candidateTerm
			try
				if (id of candidateTerm as text) is not (id of managedTerm as text) then return candidateTerm
			end try
		end repeat
	end tell

	return missing value
end anchorTerminalFor

on createPreviewSplit(anchorTerm, targetWindow, cfg, managedTitle, attachCommand)
	tell application "Ghostty"
		set newTerm to split anchorTerm direction left with configuration cfg
	end tell

	my labelManagedTerminal(newTerm, managedTitle)
	my sendAttachCommand(newTerm, attachCommand)
	tell application "Ghostty" to focus newTerm
	my resizePreview(anchorTerm, targetWindow)
	return newTerm
end createPreviewSplit

on replacePreviewSplit(managedTerm, cfg, managedTitle, attachCommand)
	tell application "Ghostty"
		set newTerm to split managedTerm direction right with configuration cfg
	end tell

	my labelManagedTerminal(newTerm, managedTitle)
	tell application "Ghostty"
		close managedTerm
		focus newTerm
	end tell
	my sendAttachCommand(newTerm, attachCommand)
	return newTerm
end replacePreviewSplit

on run argv
	if (count of argv) < 7 then error "ghostty-open requires 7 arguments"

	set targetSessionId to item 1 of argv
	set targetTmuxName to item 2 of argv
	set targetCwd to item 3 of argv
	set attachCommand to item 4 of argv
	set targetDisplayName to item 5 of argv
	set managedTitlePrefix to item 6 of argv
	set openMode to item 7 of argv
	set knownManagedId to ""
	if (count of argv) > 7 then set knownManagedId to item 8 of argv
	set managedTitle to managedTitlePrefix & targetDisplayName

	tell application "Ghostty"
		activate

		set cfg to new surface configuration
		if targetCwd is not "" then set initial working directory of cfg to targetCwd

		if (count of windows) = 0 then
			set targetWindow to new window with configuration cfg
			set targetTab to selected tab of targetWindow
			set newTerm to focused terminal of targetTab
			my labelManagedTerminal(newTerm, managedTitle)
			my sendAttachCommand(newTerm, attachCommand)
			focus newTerm
			return "created|" & (id of newTerm as text)
		end if

		set targetWindow to front window
		set targetTab to selected tab of targetWindow
	end tell

	set managedTerm to missing value
	if openMode is "swap" then
		set managedTerm to my terminalById(targetTab, knownManagedId)
		if managedTerm is missing value then
			set managedTerm to my firstManagedTerminal(targetTab, managedTitlePrefix)
		end if
	end if

	set anchorTerm to my anchorTerminalFor(targetTab, managedTerm)
	if anchorTerm is missing value then
		tell application "Ghostty" to set anchorTerm to focused terminal of targetTab
	end if

	if openMode is "add" then
		set newTerm to my createPreviewSplit(anchorTerm, targetWindow, cfg, managedTitle, attachCommand)
		return "created|" & (id of newTerm as text)
	end if

	if managedTerm is missing value then
		set newTerm to my createPreviewSplit(anchorTerm, targetWindow, cfg, managedTitle, attachCommand)
		return "created|" & (id of newTerm as text)
	end if

	set newTerm to my replacePreviewSplit(managedTerm, cfg, managedTitle, attachCommand)
	return "created|" & (id of newTerm as text)
end run
