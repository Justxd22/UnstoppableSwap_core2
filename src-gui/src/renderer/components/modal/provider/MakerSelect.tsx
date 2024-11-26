import { Box, Card, CardContent, IconButton } from "@mui/material";
import makeStyles from '@mui/styles/makeStyles';
import ArrowForwardIosIcon from "@mui/icons-material/ArrowForwardIos";
import { useState } from "react";
import { useAppSelector } from "store/hooks";
import MakerInfo from "./MakerInfo";
import MakerListDialog from "./MakerListDialog";

const useStyles = makeStyles({
  inner: {
    textAlign: "left",
    width: "100%",
    height: "100%",
  },
  makerCard: {
    width: "100%",
  },
  makerCardContent: {
    display: "flex",
    alignItems: "center",
  },
});

export default function MakerSelect() {
  const classes = useStyles();
  const [selectDialogOpen, setSelectDialogOpen] = useState(false);
  const selectedMaker = useAppSelector(
    (state) => state.makers.selectedMaker,
  );

  if (!selectedMaker) return <>No maker selected</>;

  function handleSelectDialogClose() {
    setSelectDialogOpen(false);
  }

  function handleSelectDialogOpen() {
    setSelectDialogOpen(true);
  }

  return (
    <Box>
      <MakerListDialog
        open={selectDialogOpen}
        onClose={handleSelectDialogClose}
      />
      <Card className={classes.makerCard}>
        <CardContent className={classes.makerCardContent}>
          <MakerInfo maker={selectedMaker} />
          <IconButton onClick={handleSelectDialogOpen} size="small">
            <ArrowForwardIosIcon />
          </IconButton>
        </CardContent>
      </Card>
    </Box>
  );
}
