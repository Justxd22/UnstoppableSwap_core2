import {
  Box,
  Paper,
  Table,
  TableBody,
  TableCell,
  TableContainer,
  TableHead,
  TableRow,
} from "@mui/material";
import makeStyles from '@mui/styles/makeStyles';
import { sortBy } from "lodash";
import { parseDateString } from "utils/parseUtils";
import {
  useAppSelector,
  useSwapInfosSortedByDate,
} from "../../../../../store/hooks";
import HistoryRow from "./HistoryRow";

const useStyles = makeStyles((theme) => ({
  outer: {
    paddingTop: theme.spacing(1),
    paddingBottom: theme.spacing(1),
  },
}));

export default function HistoryTable() {
  const classes = useStyles();
  const swapSortedByDate = useSwapInfosSortedByDate();

  return (
    <Box className={classes.outer}>
      <TableContainer component={Paper}>
        <Table>
          <TableHead>
            <TableRow>
              <TableCell />
              <TableCell>ID</TableCell>
              <TableCell>Amount</TableCell>
              <TableCell>State</TableCell>
              <TableCell />
            </TableRow>
          </TableHead>
          <TableBody>
            {swapSortedByDate.map((swap) => (
              <HistoryRow swap={swap} key={swap.swap_id} />
            ))}
          </TableBody>
        </Table>
      </TableContainer>
    </Box>
  );
}
