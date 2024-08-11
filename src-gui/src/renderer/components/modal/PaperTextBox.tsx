import { Button, Paper, Typography } from "@mui/material";

import makeStyles from "@mui/styles/makeStyles";

const useStyles = makeStyles((theme) => ({
  logsOuter: {
    overflow: "auto",
    padding: theme.spacing(1),
    marginTop: theme.spacing(1),
    marginBottom: theme.spacing(1),
    maxHeight: "10rem",
  },
  copyButton: {
    marginTop: theme.spacing(1),
  },
}));

export default function PaperTextBox({ stdOut }: { stdOut: string }) {
  const classes = useStyles();

  function handleCopyLogs() {
    throw new Error("Not implemented");
  }

  return (
    <Paper className={classes.logsOuter} variant="elevation">
      <Typography component="pre" variant="body2">
        {stdOut}
      </Typography>
      <Button onClick={handleCopyLogs} className={classes.copyButton}>
        Copy
      </Button>
    </Paper>
  );
}
