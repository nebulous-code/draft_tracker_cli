# Draft Tracker

I want a program that will track my historic writing word count in a sqlite database and will accept input via commandline tui. 

This should be written in rust.

A one time script should be provided to convert the existing .ods files taht are in historic_trackers/ 

The columns/data that is important to collect from the files are: 
- Date
- Project
- Day Word Count (WC)
- Hours
- Project Word Count (WC)
- Notes

All other columns on the work sheet can be derived. Some other columns exist but they feed the stats page that will be rewritten at a later date. 
I should have the ability to add new projects and historic projects should be imported automatically. 
Projects should calculate their total word count based on the day word count submitted via trackers. 
Projects need the following metadata 
- Title (required): Name of the project
- Project Category: fk to Project Category below
- Status: open, completed or abandoned, so I can indicate I'm done with a project or that I gave up on it without shipping 
- Description: (optional) long from text entry

Calculated Metadata that should be presented on the front end:
- Start Date: first day with a writing entry
- End Date: if the status is completed or abandoned then return last dated entry
- Total Word Count: running total of the project's word count
- Days Written: The count of distinct days I've written on a project
- Hours Written: The sum of the hours written on the project as a running total
- Hours per Day: The Hours Written divided by the days written to give me an idea of how long I spend on this project per day
- Words per hour: The Total Word Count for the project divided by the Hours written on the project

Project Category should be from a set list defined category table. It should be the distinct list of all Project entries over the years. I should also be able to create a new one through the UI. It needs the field: 
- Category: the project's name
- Description: (optional) free form text entry describing the category


Data Input:
I want the program to calculate how many words I've written in any given session because it's easier for me to tell the total word count of a story at any given time rather than it is to tell the number of new words across chapters. 
A daily entry via the tui should consist of: 
- Date: The date I'm adding an entry for, default to today allow me to put in past/future dates
    - Assume I will put in multiple entries for any given day potentially for the same project (although it will be more common for me to do multiple projects in one day)
- Project: selected from a list of not complete projects
- Hours: The number of hours written, free form decimal in hours (.25 is 15 mins, etc). Minimum .25, and zero is not accepted: a minute spent counts as 15 for word count accounting.
- Project Word Count: The total word count of the project Session Word Count: READ ONLY This is the difference between Project Word Count and the sum of past entries for the project to give me an idea of how much I've written for the day. This is what will be inserted into the database. It's not editable by me. But it should be presented incase I put in 20000 instead of 2000
- Words per Hour: READ ONLY the session word count divided by the hours written. Should naturally normalize to per hour. So 100 words written for .25 hours should be 400 words per hour. 
- Submit button: Short cut to <leader> + enter

Stats:
I would like a basic stats screen that accumulates across all projects. This should be at a year and lifetime level. I've put Lifetime and Current Year as prefixes but the KPIs should land in a clear section titled "Lifetime" or "Current Year" with just the rest of the field's title as the kpi header. Current year should be done off of the machine's date time.

Here's what I'm looking for: 
- Lifetime Words: Total words written according to all database entries 
- Lifetime Hours: Total hours written according to all database entries
- Lifetime Completed Projects: Count of projects that are marked as completed
- Current Year Words: Total Words Written according to all database entries marked for this year
- Current Year Hours: Total hours written according to all database entries marked for this year
- Current Year Completed Projects: Count of projects that are marked as completed and have an end date within this current year
- Current Year Projects Graph: A horizontal bar graph that shows the current year's projects (every project with an entry for the current year) their title on the left (cut at say 10 characters) and their word count on the right. If the project is marked complete color the bar green.
- Year over Year Word Count: A horizontal bar graph that shows the years on the left and total word count on the right with a bar graph between to visually compare
- Current Year Project Category Word Count Distribution: this is a pie chart expressed through a single bar. It will find the total words written for the year and then divide each category's word count for the year by that to find a percentage and then fill in the bar by that percentage.

Additional Note: I would like this dashboard written in a way so that the current year could be modified to show a previous year. Whether or not that is implemented in the mvp is up for discussion but I'd like the functionality and the way the backend queries are written to be year agnostic. 

Navigation and Presentation

I prefer vim style keybindings over most anything else so give me those. A popup menu to remind me of the keyboard shortcuts with ? would be idea. Avoid using ctrl+<key> where possible since that doesn't translate across mac/linux/windows. I use space bar as <leader> in vim. 

I use alacritty or Kitty as my terminal emulator so we can work with full color. 
Colors should be based on the nvim melange color scheme listed below:

return {
  a = {
    bg      = "#292522",
    float   = "#34302C",
    sel     = "#403A36",
    ui      = "#867462",
    com     = "#C1A78E",
    fg      = "#ECE1D7",
  },
  b = {
    red     = "#D47766",
    yellow  = "#EBC06D",
    green   = "#85B695",
    cyan    = "#89B3B6",
    blue    = "#A3A9CE",
    magenta = "#CF9BC2",
  },
  c = {
    red     = "#BD8183",
    yellow  = "#E49B5D",
    green   = "#78997A",
    cyan    = "#7B9695",
    blue    = "#7F91B2",
    magenta = "#B380B0",
  },

  d = {
    red     = "#7D2A2F",
    yellow  = "#8B7449",
    green   = "#233524",
    cyan    = "#253333",
    blue    = "#273142",
    magenta = "#422741",
  },
}


