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

on managedTerminals(targetTab, managedTitlePrefix)
	set matches to {}

	tell application "Ghostty"
		repeat with candidateTerm in terminals of targetTab
			try
				set termTitle to name of candidateTerm
				if termTitle starts with managedTitlePrefix then set end of matches to candidateTerm
			end try
		end repeat
	end tell

	return matches
end managedTerminals

on terminalIds(targetTerms)
	set ids to {}

	tell application "Ghostty"
		repeat with candidateTerm in targetTerms
			try
				set end of ids to (id of candidateTerm as text)
			end try
		end repeat
	end tell

	return ids
end terminalIds

on preferredManagedTerminal(targetTab, knownManagedId, managedTerms)
	set knownTerm to my terminalById(targetTab, knownManagedId)
	if knownTerm is not missing value then return knownTerm
	if (count of managedTerms) > 0 then return item 1 of managedTerms
	return missing value
end preferredManagedTerminal

on closeManagedTerminals(targetTerms)
	tell application "Ghostty"
		repeat with candidateTerm in targetTerms
			try
				close candidateTerm
			end try
		end repeat
	end tell
end closeManagedTerminals

on anchorTerminalFor(targetTab, excludedIds)
	tell application "Ghostty"
		try
			set focusedTerm to focused terminal of targetTab
			set focusedId to id of focusedTerm as text
			if excludedIds does not contain focusedId then return focusedTerm
		end try

		repeat with candidateTerm in terminals of targetTab
			try
				set candidateId to id of candidateTerm as text
				if excludedIds does not contain candidateId then return candidateTerm
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

	set managedTerms to {}
	set managedTerm to missing value
	set excludedIds to {}
	if openMode is "swap" then
		set managedTerms to my managedTerminals(targetTab, managedTitlePrefix)
		set managedTerm to my preferredManagedTerminal(targetTab, knownManagedId, managedTerms)
		if managedTerm is not missing value then
			tell application "Ghostty" to set managedTermId to (id of managedTerm as text)
			set duplicateManagedTerms to {}
			repeat with candidateTerm in managedTerms
				try
					tell application "Ghostty" to set candidateId to (id of candidateTerm as text)
					if candidateId is not managedTermId then set end of duplicateManagedTerms to candidateTerm
				end try
			end repeat
			my closeManagedTerminals(duplicateManagedTerms)
			set excludedIds to {managedTermId}
		else
			set excludedIds to my terminalIds(managedTerms)
		end if
	end if

	set anchorTerm to my anchorTerminalFor(targetTab, excludedIds)
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
