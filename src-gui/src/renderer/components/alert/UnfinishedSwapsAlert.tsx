import { Button } from "@material-ui/core";
import Alert from "@material-ui/lab/Alert";
import { useNavigate } from "react-router-dom";
import { useResumeableSwapsCountExcludingPunishedAndSetup } from "store/hooks";

export default function UnfinishedSwapsAlert() {
  const resumableSwapsCount = useResumeableSwapsCountExcludingPunishedAndSetup();
  const navigate = useNavigate();

  if (resumableSwapsCount > 0) {
    return (
      <Alert
        severity="warning"
        variant="filled"
        action={
          <Button
            variant="outlined"
            size="small"
            onClick={() => navigate("/history")}
          >
            VIEW
          </Button>
        }
      >
        You have{" "}
        {resumableSwapsCount > 1
          ? `${resumableSwapsCount} unfinished swaps`
          : "one unfinished swap"}
      </Alert>
    );
  }
  return null;
}
