# Strip what differs between two runs of one binary on one fixture: wall-clock
# times and the random id of each event block. Everything else is compared.
walk(
  if type == "string" and test("^[0-9]{4}-[0-9]{2}-[0-9]{2}T[0-9]{2}:[0-9]{2}:[0-9]{2}") then "<time>"
  elif type == "string" and test("^T[0-9]{2}:[0-9]{2}:[0-9]{2}$") then "<time-of-day>"
  else . end
)
