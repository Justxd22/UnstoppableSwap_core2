import {
  Avatar,
  Button,
  Dialog,
  DialogActions,
  DialogContent,
  DialogTitle,
  List,
  ListItem,
  ListItemAvatar,
  ListItemText,
} from "@mui/material";
import makeStyles from '@mui/styles/makeStyles';
import AddIcon from "@mui/icons-material/Add";
import SearchIcon from "@mui/icons-material/Search";
import { ExtendedMakerStatus } from "models/apiModel";
import { useState } from "react";
import { setSelectedMaker } from "store/features/makersSlice";
import { useAllMakers, useAppDispatch } from "store/hooks";
import ListSellersDialog from "../listSellers/ListSellersDialog";
import MakerInfo from "./MakerInfo";
import MakerSubmitDialog from "./MakerSubmitDialog";

const useStyles = makeStyles({
  dialogContent: {
    padding: 0,
  },
});

type MakerSelectDialogProps = {
  open: boolean;
  onClose: () => void;
};

export function MakerSubmitDialogOpenButton() {
  const [open, setOpen] = useState(false);

  return (
    <ListItem
      autoFocus
      button
      onClick={() => {
        // Prevents background from being clicked and reopening dialog
        if (!open) {
          setOpen(true);
        }
      }}
    >
      <MakerSubmitDialog open={open} onClose={() => setOpen(false)} />
      <ListItemAvatar>
        <Avatar>
          <AddIcon />
        </Avatar>
      </ListItemAvatar>
      <ListItemText primary="Add a new maker to public registry" />
    </ListItem>
  );
}

export function ListSellersDialogOpenButton() {
  const [open, setOpen] = useState(false);

  return (
    <ListItem
      autoFocus
      button
      onClick={() => {
        // Prevents background from being clicked and reopening dialog
        if (!open) {
          setOpen(true);
        }
      }}
    >
      <ListSellersDialog open={open} onClose={() => setOpen(false)} />
      <ListItemAvatar>
        <Avatar>
          <SearchIcon />
        </Avatar>
      </ListItemAvatar>
      <ListItemText primary="Discover makers by connecting to a rendezvous point" />
    </ListItem>
  );
}

export default function MakerListDialog({
  open,
  onClose,
}: MakerSelectDialogProps) {
  const classes = useStyles();
  const makers = useAllMakers();
  const dispatch = useAppDispatch();

  function handleMakerChange(maker: ExtendedMakerStatus) {
    dispatch(setSelectedMaker(maker));
    onClose();
  }

  return (
    <Dialog onClose={onClose} open={open}>
      <DialogTitle>Select a maker</DialogTitle>

      <DialogContent className={classes.dialogContent} dividers>
        <List>
          {makers.map((maker) => (
            <ListItem
              button
              onClick={() => handleMakerChange(maker)}
              key={maker.peerId}
            >
              <MakerInfo maker={maker} key={maker.peerId} />
            </ListItem>
          ))}
          <ListSellersDialogOpenButton />
          <MakerSubmitDialogOpenButton />
        </List>
      </DialogContent>

      <DialogActions>
        <Button onClick={onClose}>Cancel</Button>
      </DialogActions>
    </Dialog>
  );
}
